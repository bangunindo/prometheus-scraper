//! Layer 3 — an async HTTP scrape client (feature `client` / `client-native-tls`).
//!
//! [`Client`] fetches a Prometheus / OpenMetrics endpoint and streams the parsed
//! [`owned::MetricFamily`](crate::owned::MetricFamily) values out, wrapping the
//! incremental [`Decoder`](crate::Decoder) over a reqwest response body. Build
//! one with [`Client::builder`] and reuse it for many scrapes:
//!
//! ```no_run
//! # async fn run() -> Result<(), prometheus_scraper::ScrapeError> {
//! use futures_util::StreamExt;
//! let client = prometheus_scraper::Client::builder("https://host:9100/metrics")
//!     .bearer_token("secret")
//!     .build()?;
//! loop {
//!     let mut families = client.scrape().await?;
//!     while let Some(family) = families.next().await {
//!         let family = family?;
//!         // … do something with `family` …
//!     }
//!     tokio::time::sleep(std::time::Duration::from_secs(5)).await;
//! }
//! # }
//! ```
//!
//! The hostname is **re-resolved on every `scrape()`** (the underlying client
//! keeps no idle connection pool), so a load-balanced / round-robin DNS name can
//! land on a different backend each time.

use std::fmt;
use std::pin::Pin;
use std::time::Duration;

use bytes::Bytes;
use futures_core::Stream;
use futures_util::{StreamExt, TryStreamExt};
use reqwest::header::{ACCEPT, CONTENT_TYPE};

use crate::owned;
use crate::{Decoder, Format, ParseError, TextFormat};

/// Re-exported from `reqwest` so callers can construct TLS material with the
/// constructor that matches their enabled backend (e.g. [`Identity::from_pem`]
/// for rustls, `Identity::from_pkcs12_der` for native-tls).
pub use reqwest::{Certificate, Identity};

/// The stream of parsed families returned by [`Client::scrape`]. Boxed and
/// pinned so it is [`Unpin`] — call [`StreamExt::next`](futures_util::StreamExt)
/// on it directly without pinning yourself.
pub type FamilyStream =
    Pin<Box<dyn Stream<Item = Result<owned::MetricFamily, ScrapeError>> + Send>>;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Anything that can go wrong during a scrape. [`Parse`](ScrapeError::Parse)
/// wraps the crate's [`ParseError`]; the rest are transport / protocol issues.
#[derive(Debug)]
#[non_exhaustive]
pub enum ScrapeError {
    /// Building the underlying HTTP client failed (e.g. invalid TLS material).
    Build(reqwest::Error),
    /// The request failed: connect, TLS handshake, timeout, or body I/O.
    Http(reqwest::Error),
    /// The endpoint answered with a non-success HTTP status.
    Status(reqwest::StatusCode),
    /// A metric family in the response failed to parse.
    Parse(ParseError),
}

impl fmt::Display for ScrapeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScrapeError::Build(e) => write!(f, "failed to build HTTP client: {e}"),
            ScrapeError::Http(e) => write!(f, "scrape request failed: {e}"),
            ScrapeError::Status(s) => write!(f, "scrape returned HTTP status {s}"),
            ScrapeError::Parse(e) => write!(f, "failed to parse scraped metrics: {e}"),
        }
    }
}

impl std::error::Error for ScrapeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ScrapeError::Build(e) | ScrapeError::Http(e) => Some(e),
            ScrapeError::Parse(e) => Some(e),
            ScrapeError::Status(_) => None,
        }
    }
}

impl From<ParseError> for ScrapeError {
    fn from(e: ParseError) -> Self {
        ScrapeError::Parse(e)
    }
}

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

#[derive(Clone)]
enum Auth {
    None,
    Basic { username: String, password: String },
    Bearer { token: String },
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Configures and builds a [`Client`]. Created via [`Client::builder`].
pub struct ClientBuilder {
    url: String,
    auth: Auth,
    timeout: Option<Duration>,
    connect_timeout: Option<Duration>,
    format: Option<Format>,
    accept_protobuf: bool,
    identity: Option<Identity>,
    root_certs: Vec<Certificate>,
    accept_invalid_certs: bool,
}

impl ClientBuilder {
    fn new(url: impl Into<String>) -> Self {
        ClientBuilder {
            url: url.into(),
            auth: Auth::None,
            timeout: None,
            connect_timeout: None,
            format: None,
            accept_protobuf: false,
            identity: None,
            root_certs: Vec::new(),
            accept_invalid_certs: false,
        }
    }

    /// Authenticate with HTTP Basic auth.
    pub fn basic_auth(mut self, username: impl Into<String>, password: impl Into<String>) -> Self {
        self.auth = Auth::Basic {
            username: username.into(),
            password: password.into(),
        };
        self
    }

    /// Authenticate with a bearer token (`Authorization: Bearer …`).
    pub fn bearer_token(mut self, token: impl Into<String>) -> Self {
        self.auth = Auth::Bearer {
            token: token.into(),
        };
        self
    }

    /// Overall per-request timeout (connect + response). See
    /// [`reqwest::ClientBuilder::timeout`].
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Connect-phase timeout only. See [`reqwest::ClientBuilder::connect_timeout`].
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = Some(timeout);
        self
    }

    /// Force a specific exposition [`Format`], skipping `Content-Type`-based
    /// negotiation. Useful when an endpoint mislabels or omits its `Content-Type`.
    pub fn format(mut self, format: Format) -> Self {
        self.format = Some(format);
        self
    }

    /// Also advertise the protobuf exposition format in the `Accept` header
    /// (off by default; most exporters serve text). When enabled and the server
    /// responds with protobuf, native/hybrid histograms become available.
    pub fn accept_protobuf(mut self, yes: bool) -> Self {
        self.accept_protobuf = yes;
        self
    }

    /// Present a client certificate for mutual TLS. Construct the [`Identity`]
    /// with the constructor matching your TLS backend.
    pub fn identity(mut self, identity: Identity) -> Self {
        self.identity = Some(identity);
        self
    }

    /// Trust an additional root [`Certificate`] (e.g. a private/internal CA).
    /// May be called multiple times.
    pub fn add_root_certificate(mut self, cert: Certificate) -> Self {
        self.root_certs.push(cert);
        self
    }

    /// **Dangerous.** Accept invalid/self-signed TLS certificates. Disables
    /// authentication of the server — only for testing or trusted networks.
    pub fn danger_accept_invalid_certs(mut self, accept: bool) -> Self {
        self.accept_invalid_certs = accept;
        self
    }

    /// Build the reusable [`Client`].
    pub fn build(self) -> Result<Client, ScrapeError> {
        // pool_max_idle_per_host(0) keeps no idle connections, so every scrape
        // opens a fresh connection and re-resolves DNS via the system resolver.
        let mut builder = reqwest::Client::builder().pool_max_idle_per_host(0);
        if let Some(t) = self.timeout {
            builder = builder.timeout(t);
        }
        if let Some(t) = self.connect_timeout {
            builder = builder.connect_timeout(t);
        }
        if let Some(identity) = self.identity {
            builder = builder.identity(identity);
        }
        for cert in self.root_certs {
            builder = builder.add_root_certificate(cert);
        }
        if self.accept_invalid_certs {
            builder = builder.danger_accept_invalid_certs(true);
        }
        let http = builder.build().map_err(ScrapeError::Build)?;
        Ok(Client {
            http,
            url: self.url,
            auth: self.auth,
            accept: accept_header(self.accept_protobuf),
            format: self.format,
        })
    }
}

/// Build the `Accept` header value, in preference order.
fn accept_header(protobuf: bool) -> String {
    let mut value = String::new();
    if protobuf {
        value.push_str(
            "application/vnd.google.protobuf; \
             proto=io.prometheus.client.MetricFamily; encoding=delimited, ",
        );
    }
    value.push_str(
        "application/openmetrics-text; version=1.0.0, \
         text/plain; version=0.0.4; q=0.9, */*; q=0.1",
    );
    value
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// A reusable async scrape client. Cheap to clone (the inner reqwest client is
/// reference-counted). See the [module docs](self) for an example.
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    url: String,
    auth: Auth,
    accept: String,
    format: Option<Format>,
}

impl Client {
    /// Start configuring a client for the given endpoint URL.
    pub fn builder(url: impl Into<String>) -> ClientBuilder {
        ClientBuilder::new(url)
    }

    /// Scrape the endpoint and stream parsed families as they arrive.
    ///
    /// Errors are per-family: a parse failure yields one `Err` item and the
    /// stream continues at the next family (the framing resyncs), while a
    /// transport error ends the stream. The hostname is resolved fresh for this
    /// call.
    pub async fn scrape(&self) -> Result<FamilyStream, ScrapeError> {
        let response = self.send().await?;
        let format = self
            .format
            .unwrap_or_else(|| detect_format(response.headers()));
        Ok(Box::pin(decode_stream(response.bytes_stream(), format)))
    }

    /// Scrape the endpoint and collect every family into a `Vec`.
    ///
    /// Unlike [`scrape`](Self::scrape), this fails on the first error instead of
    /// surfacing per-family results.
    pub async fn scrape_all(&self) -> Result<Vec<owned::MetricFamily>, ScrapeError> {
        self.scrape().await?.try_collect().await
    }

    /// Issue the GET (auth + `Accept` applied), returning the response once the
    /// status line and headers have arrived.
    async fn send(&self) -> Result<reqwest::Response, ScrapeError> {
        let mut request = self.http.get(&self.url).header(ACCEPT, &self.accept);
        request = match &self.auth {
            Auth::None => request,
            Auth::Basic { username, password } => request.basic_auth(username, Some(password)),
            Auth::Bearer { token } => request.bearer_auth(token),
        };
        let response = request.send().await.map_err(ScrapeError::Http)?;
        if !response.status().is_success() {
            return Err(ScrapeError::Status(response.status()));
        }
        Ok(response)
    }
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never print credentials.
        let auth = match self.auth {
            Auth::None => "none",
            Auth::Basic { .. } => "basic",
            Auth::Bearer { .. } => "bearer",
        };
        f.debug_struct("Client")
            .field("url", &self.url)
            .field("auth", &auth)
            .field("format", &self.format)
            .finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// Format negotiation & body decoding
// ---------------------------------------------------------------------------

/// Resolve the exposition [`Format`] from the response `Content-Type`.
///
/// A recognized type maps to a concrete dialect; anything missing or unknown
/// (the server ignored our `Accept`) falls back to [`TextFormat::Guess`], since
/// we can no longer trust the declared dialect.
fn detect_format(headers: &reqwest::header::HeaderMap) -> Format {
    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if content_type.contains("application/openmetrics-text") {
        Format::Text(TextFormat::OpenMetrics)
    } else if content_type.contains("application/vnd.google.protobuf") {
        Format::Protobuf
    } else if content_type.contains("text/plain") {
        Format::Text(TextFormat::Prometheus)
    } else {
        Format::Text(TextFormat::Guess)
    }
}

/// Drive the incremental [`Decoder`] over a response body stream, yielding owned
/// families. Factored out of [`Client::scrape`] so it can be tested with a
/// fabricated body and no network.
fn decode_stream(
    body: impl Stream<Item = Result<Bytes, reqwest::Error>>,
    format: Format,
) -> impl Stream<Item = Result<owned::MetricFamily, ScrapeError>> {
    async_stream::stream! {
        let mut decoder = Decoder::new(format);
        futures_util::pin_mut!(body);
        while let Some(chunk) = body.next().await {
            match chunk {
                Ok(bytes) => decoder.push(&bytes),
                Err(e) => {
                    yield Err(ScrapeError::Http(e));
                    return;
                }
            }
            while let Some(family) = decoder.next_owned() {
                yield family.map_err(ScrapeError::Parse);
            }
        }
        decoder.finish();
        while let Some(family) = decoder.next_owned() {
            yield family.map_err(ScrapeError::Parse);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::owned::MetricType;

    fn header_map(content_type: &str) -> reqwest::header::HeaderMap {
        let mut h = reqwest::header::HeaderMap::new();
        h.insert(CONTENT_TYPE, content_type.parse().unwrap());
        h
    }

    #[test]
    fn detects_format_from_content_type() {
        assert_eq!(
            detect_format(&header_map("application/openmetrics-text; version=1.0.0")),
            Format::Text(TextFormat::OpenMetrics)
        );
        assert_eq!(
            detect_format(&header_map("text/plain; version=0.0.4; charset=utf-8")),
            Format::Text(TextFormat::Prometheus)
        );
        assert_eq!(
            detect_format(&header_map(
                "application/vnd.google.protobuf; encoding=delimited"
            )),
            Format::Protobuf
        );
        // Mixed case is normalized.
        assert_eq!(
            detect_format(&header_map("Application/OpenMetrics-Text")),
            Format::Text(TextFormat::OpenMetrics)
        );
        // Unknown / missing => Guess.
        assert_eq!(
            detect_format(&header_map("application/json")),
            Format::Text(TextFormat::Guess)
        );
        assert_eq!(
            detect_format(&reqwest::header::HeaderMap::new()),
            Format::Text(TextFormat::Guess)
        );
    }

    const PAYLOAD: &str = "# TYPE a counter\na_total 1\n\
                           # TYPE b gauge\nb 2\n\
                           # TYPE c counter\nc_total 3\n";

    /// Feed the payload through `decode_stream` split at a given chunk size and
    /// collect the family names.
    async fn names_for_chunk_size(chunk: usize) -> Vec<String> {
        let bytes = PAYLOAD.as_bytes();
        let parts: Vec<Result<Bytes, reqwest::Error>> = bytes
            .chunks(chunk)
            .map(|c| Ok(Bytes::copy_from_slice(c)))
            .collect();
        let body = futures_util::stream::iter(parts);
        decode_stream(body, Format::Text(TextFormat::OpenMetrics))
            .map(|r| r.unwrap().name)
            .collect()
            .await
    }

    #[tokio::test]
    async fn decode_stream_is_chunk_invariant() {
        for chunk in 1..=PAYLOAD.len() {
            let names = names_for_chunk_size(chunk).await;
            assert_eq!(names, ["a", "b", "c"], "chunk size {chunk}");
        }
    }

    #[tokio::test]
    async fn parse_error_is_isolated_in_stream() {
        let payload = "# TYPE a gauge\na 1\n# TYPE b gauge\nb nope\n# TYPE c gauge\nc 3\n";
        let body = futures_util::stream::iter(vec![Ok::<_, reqwest::Error>(Bytes::from(payload))]);
        let results: Vec<_> = decode_stream(body, Format::Text(TextFormat::OpenMetrics))
            .collect()
            .await;
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].as_ref().unwrap().name, "a");
        assert!(matches!(results[1], Err(ScrapeError::Parse(_))));
        assert_eq!(results[2].as_ref().unwrap().name, "c");
    }

    #[tokio::test]
    async fn end_to_end_over_tcp_with_auth() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        // A one-shot server: read the request, assert the bearer header, reply.
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 1024];
            let n = sock.read(&mut buf).await.unwrap();
            let request = String::from_utf8_lossy(&buf[..n]).to_string();
            let body = "# TYPE up gauge\nup 1\n# TYPE reqs counter\nreqs_total 7\n";
            let response = format!(
                "HTTP/1.1 200 OK\r\n\
                 Content-Type: application/openmetrics-text; version=1.0.0\r\n\
                 Content-Length: {}\r\n\
                 Connection: close\r\n\r\n{}",
                body.len(),
                body
            );
            sock.write_all(response.as_bytes()).await.unwrap();
            sock.flush().await.unwrap();
            request
        });

        let client = Client::builder(format!("http://{addr}/metrics"))
            .bearer_token("s3cret")
            .build()
            .unwrap();
        let families = client.scrape_all().await.unwrap();

        let request = server.await.unwrap();
        assert!(
            request.contains("authorization: Bearer s3cret")
                || request.contains("Authorization: Bearer s3cret"),
            "request must carry the bearer token, got:\n{request}"
        );
        assert_eq!(families.len(), 2);
        assert_eq!(families[0].name, "up");
        assert_eq!(families[0].r#type, MetricType::Gauge);
        assert_eq!(families[1].name, "reqs");
        assert_eq!(families[1].r#type, MetricType::Counter);
    }
}
