# prometheus-scraper

[![Crates.io](https://img.shields.io/crates/v/prometheus-scraper.svg)](https://crates.io/crates/prometheus-scraper)
[![Documentation](https://docs.rs/prometheus-scraper/badge.svg)](https://docs.rs/prometheus-scraper)
[![License](https://img.shields.io/crates/l/prometheus-scraper.svg)](#license)

Parse the Prometheus / OpenMetrics exposition formats — both the **text** and the
**protobuf** dialects — into typed Rust values, with an optional async client that
scrapes an endpoint and streams the results.

- **Both formats, one API.** Classic Prometheus text, OpenMetrics text, and the
  protobuf exposition format, selected with a single [`Format`] argument.
- **Zero-copy parsing.** Parsed families borrow straight from the input buffer;
  call `into_owned()` when you need to detach from it.
- **Streaming, Sans-I/O.** Feed bytes as they arrive with the [`Decoder`] and pull
  families out as they complete — no buffering the whole body, no I/O assumptions.
- **Resilient.** A malformed family yields one error and parsing resumes at the
  next one, so a single bad metric never discards the rest of a scrape.
- **Native & hybrid histograms.** The protobuf format exposes exponential native
  histograms (and the hybrid classic-plus-native form) that text cannot represent.
- **Optional async client.** Scrape an HTTP endpoint with auth, mTLS, and
  content-type negotiation behind a feature flag.

## Installation

```toml
[dependencies]
prometheus-scraper = "0.1"
```

To also pull in the async scrape client, enable the `client` feature:

```toml
[dependencies]
prometheus-scraper = { version = "0.1", features = ["client"] }
```

## Quick start

Parse a whole scrape body — a *payload*, which is just a sequence of metric
families:

```rust
use prometheus_scraper::{parse_payload, Format, ParseError, TextFormat};

fn main() -> Result<(), ParseError> {
    let body = b"# TYPE http_requests counter\n\
                 http_requests_total{code=\"200\"} 1027\n";

    for family in parse_payload(body, Format::Text(TextFormat::Prometheus)) {
        let family = family?; // each family parses independently
        println!("{} ({:?})", family.name, family.r#type);
        for metric in &family.metric {
            println!("  {:?} = {:?}", metric.label, metric.value);
        }
    }
    Ok(())
}
```

Each family borrows from `body` (zero-copy). When you need a value that outlives
the buffer, convert it with `into_owned()`:

```rust
let owned: Vec<_> = parse_payload(body, Format::Text(TextFormat::Prometheus))
    .map(|family| family.map(|f| f.into_owned()))
    .collect::<Result<_, _>>()?;
```

## Streaming with the `Decoder`

When bytes arrive in chunks (from a socket, a decompressor, …), drive the
incremental [`Decoder`] instead. It is Sans-I/O: you push bytes, it hands back
families as soon as they are complete, and it is invariant to where the chunk
boundaries fall.

```rust
use prometheus_scraper::{Decoder, Format, TextFormat};

let mut decoder = Decoder::new(Format::Text(TextFormat::OpenMetrics));

// Feed arbitrary chunks as they arrive…
decoder.push(b"# TYPE a counter\na_to");
decoder.push(b"tal 1\n# TYPE b gauge\nb 2\n");
decoder.finish(); // no more input is coming

for family in decoder.iter_owned() {
    let family = family?;
    println!("{}", family.name);
}
```

Use `next_family()` for a borrowed (lending) view that avoids the per-family
allocation, or `next_owned()` / `iter_owned()` to collect across calls.

## Async scrape client

With the `client` feature, [`Client`] fetches an endpoint and streams the parsed
families. It negotiates the exposition format from the response `Content-Type`,
re-resolves DNS on every scrape, and supports basic / bearer auth and TLS client
certificates.

```rust
use futures_util::StreamExt;
use prometheus_scraper::Client;

#[tokio::main]
async fn main() -> Result<(), prometheus_scraper::ScrapeError> {
    let client = Client::builder("https://host:9100/metrics")
        .bearer_token("secret")
        .accept_protobuf(true) // also advertise protobuf (for native histograms)
        .build()?;

    let mut families = client.scrape().await?;
    while let Some(family) = families.next().await {
        let family = family?;
        println!("{}", family.name);
    }
    Ok(())
}
```

`scrape()` streams `Result` items so a parse error in one family does not abort
the rest; use `scrape_all()` to collect into a `Vec`, failing on the first error.

## The data model

Every parser returns a [`borrowed::MetricFamily`] (strings borrow from the input);
[`MetricFamily::into_owned`] produces the matching [`owned::MetricFamily`] with all
data copied. The two modules mirror each other field-for-field.

A family groups metrics that share a name, type, and metadata. Each metric carries
a label set, an optional timestamp, and a typed value:

| `MetricType` | `MetricValue` variant | Notes |
| --- | --- | --- |
| `Counter` | `Counter` | value, optional exemplar, `_created` |
| `Gauge` | `Gauge` | |
| `Summary` | `Summary` | quantiles, sum, count |
| `Histogram` | `Histogram` | classic explicit buckets |
| `GaugeHistogram` | `GaugeHistogram` | buckets that can decrease |
| `NativeHistogram` | `NativeHistogram` | exponential, sparse buckets (protobuf only) |
| `HybridHistogram` | `HybridHistogram` | classic **and** native at once (protobuf only) |
| `StateSet` | `StateSet` | |
| `Info` | `Info` | |
| unknown / absent | `Untyped` | |

A protobuf `Histogram` resolves to `Histogram`, `NativeHistogram`, or
`HybridHistogram` depending on what the message actually carries; the native and
hybrid forms are protobuf-only — text parsing never produces them.
Numeric values keep the integer-vs-float distinction they were written with (see
[`Number`] / [`UnsignedNumber`]).

## Feature flags

| Feature | Default | Description |
| --- | :---: | --- |
| `chrono` | ✅ | Timestamps as [`chrono::DateTime<Utc>`]. Without it, timestamps are a plain `owned::Timestamp` (`seconds` + `nanos`) and the only dependencies are the parser crates. |
| `serde` | | Serialize borrowed and owned metric models, and deserialize the owned model. |
| `client` | | The async [`Client`], built on `reqwest` with the portable `rustls` TLS backend. |
| `client-native-tls` | | The same client over the platform's native TLS stack instead of `rustls`. |

## License

Licensed under either of

* Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
* MIT license ([LICENSE-MIT](LICENSE-MIT) or
  <http://opensource.org/licenses/MIT>)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.

[`Format`]: https://docs.rs/prometheus-scraper/latest/prometheus_scraper/payload/enum.Format.html
[`Decoder`]: https://docs.rs/prometheus-scraper/latest/prometheus_scraper/payload/struct.Decoder.html
[`Client`]: https://docs.rs/prometheus-scraper/latest/prometheus_scraper/client/struct.Client.html
[`borrowed::MetricFamily`]: https://docs.rs/prometheus-scraper/latest/prometheus_scraper/borrowed/struct.MetricFamily.html
[`MetricFamily::into_owned`]: https://docs.rs/prometheus-scraper/latest/prometheus_scraper/borrowed/struct.MetricFamily.html#method.into_owned
[`owned::MetricFamily`]: https://docs.rs/prometheus-scraper/latest/prometheus_scraper/owned/struct.MetricFamily.html
[`Number`]: https://docs.rs/prometheus-scraper/latest/prometheus_scraper/owned/enum.Number.html
[`UnsignedNumber`]: https://docs.rs/prometheus-scraper/latest/prometheus_scraper/owned/enum.UnsignedNumber.html
[`chrono::DateTime<Utc>`]: https://docs.rs/chrono/latest/chrono/struct.DateTime.html
