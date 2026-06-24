//! A permissive parser for the Prometheus / OpenMetrics text exposition format.
//!
//! [`parse_family`] turns one already-separated metric-family block of text into
//! a [`borrowed::MetricFamily`](MetricFamily). [`parse_header`]
//! handles just the leading `# TYPE` / `# HELP` / `# UNIT` lines.
//!
//! The parser is deliberately the *union* of every Prometheus and OpenMetrics
//! text dialect, leaning to the most permissive reading wherever the specs
//! disagree:
//!
//! * **Names** may be bare (`foo_seconds`), double-quoted UTF-8
//!   (`# TYPE "my.metric" gauge`), or given inside the braces in the
//!   Prometheus-3.0 form (`{"my.metric", room="k"} 1`). Bare names are taken up
//!   to the next whitespace or `{`, so dotted/UTF-8 names emitted unquoted by
//!   lenient exporters still parse.
//! * **Keywords** (`TYPE`/`HELP`/`UNIT`) are uppercase, as every spec requires;
//!   the type *value* is matched case-insensitively. `untyped` (classic) and
//!   `unknown` (OpenMetrics) both map to [`MetricType::Untyped`], as does any
//!   unrecognized keyword.
//! * **Counters** accept both the classic bare sample (`foo 5`) and the
//!   OpenMetrics `foo_total` suffix, plus an optional `foo_created`.
//! * **Numbers** are recorded as integers when their text has no decimal point
//!   or exponent (the OpenMetrics rule), otherwise as floats; `NaN`/`Inf` are
//!   accepted case-insensitively with an optional sign.
//! * **Escapes** in quoted names, label values and HELP resolve `\\`, `\n` and
//!   `\"` (the OpenMetrics superset; classic HELP only ever uses the first two).
//! * **Timestamps** are unit-correct per the [`TextFormat`] passed to
//!   [`parse_family`]: classic Prometheus trailing timestamps are milliseconds,
//!   OpenMetrics timestamps are seconds, and [`TextFormat::Guess`] auto-detects
//!   each one (a fractional value is unambiguously seconds; a plain integer is
//!   resolved by proximity to the current time). `_created` and exemplar
//!   timestamps are always seconds (both are OpenMetrics-only features).
//!
//! Native and hybrid histograms cannot be represented in text (they are
//! protobuf-only), so [`parse_family`] never produces those variants.

use std::borrow::Cow;
use std::collections::HashMap;

use nom::{
    Err, IResult, Parser,
    branch::alt,
    bytes::complete::{tag, take_till, take_till1},
    character::complete::{char, line_ending, space0, space1},
    combinator::{map, opt},
    error::{Error as NomError, ErrorKind},
    multi::many0,
    sequence::{delimited, preceded},
};

use crate::ParseError;
use crate::borrowed::{
    BucketCount, BucketFloat, BucketInt, Counter, Exemplar, Histogram, Info, LabelPair, Metric,
    MetricFamily, MetricValue, State, StateSet,
};
use crate::owned::{self, MetricType};

pub(crate) mod scan;

/// The crate's timestamp representation, mirroring the `borrowed`/`owned`
/// structs: a `chrono` instant when the feature is on, plain seconds+nanos
/// otherwise.
#[cfg(not(feature = "chrono"))]
type Ts = owned::Timestamp;
#[cfg(feature = "chrono")]
type Ts = chrono::DateTime<chrono::Utc>;

/// How to interpret the unit of a trailing sample timestamp. The dialects only
/// diverge here; the grammar accepted is otherwise the permissive union of both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextFormat {
    /// Classic Prometheus text format (`text/plain; version=0.0.4`). Trailing
    /// sample timestamps are integer milliseconds since the Unix epoch.
    Prometheus,
    /// OpenMetrics text format (`application/openmetrics-text; version=1.0.0`).
    /// Timestamps are (possibly fractional) seconds since the Unix epoch.
    OpenMetrics,
    /// Dialect unknown — resolve each trailing timestamp on its own:
    ///
    /// * A **fractional** value (decimal point or exponent) can't be a classic
    ///   `int64` millisecond timestamp, so it is read as OpenMetrics seconds
    ///   exactly — not a guess.
    /// * A plain **integer** is interpreted as *both* milliseconds and seconds,
    ///   keeping whichever lands closer to the current system time.
    ///
    /// That integer case is a heuristic — reliable for timestamps near "now",
    /// but it can misjudge values far in the past or future, and it makes
    /// parsing depend on the wall clock. Prefer [`Prometheus`](Self::Prometheus)
    /// / [`OpenMetrics`](Self::OpenMetrics) when the content type is known.
    /// `_created` and exemplar timestamps are unaffected (always seconds).
    Guess,
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Parse one already-separated metric-family block in the given [`TextFormat`].
///
/// The `format` keeps timestamp parsing correct, since the text format is
/// unit-ambiguous: classic Prometheus trailing timestamps are integer
/// milliseconds, while OpenMetrics timestamps are fractional seconds.
/// (`_created` and exemplar timestamps are always seconds — both are
/// OpenMetrics-only features.)
pub fn parse_family(text: &str, format: TextFormat) -> Result<MetricFamily<'_>, ParseError> {
    let (rest, header) = parse_header(text)
        .map_err(|_| ParseError::InvalidLine(text.lines().next().unwrap_or("").to_string()))?;
    let samples = parse_samples(rest)?;
    let family_type = header.r#type.unwrap_or(MetricType::Untyped);

    // With no descriptor lines the name comes from the first sample.
    let name: Cow<str> = match header.name {
        Some(n) => n,
        None => samples
            .first()
            .map(|s| s.name.clone())
            .unwrap_or(Cow::Borrowed("")),
    };

    let metric = assemble(format, family_type, name.as_ref(), samples);

    Ok(MetricFamily {
        name,
        help: header.help,
        r#type: family_type,
        metric,
        unit: header.unit,
    })
}

/// For the frame scanner ([`scan`]): the family name a single descriptor line
/// (`# TYPE` / `# HELP` / `# UNIT`) declares, or `None` for any other line
/// (samples, `# EOF`, plain comments, blanks).
pub(super) fn descriptor_name(line: &str) -> Option<Cow<'_, str>> {
    descriptor_line(line).ok().map(|(_, d)| d.name)
}

/// For the frame scanner ([`scan`]): the metric name of a single sample line,
/// in either the `name{…}` or `{"name",…}` form, or `None` if it doesn't parse
/// as a sample (e.g. an anonymous `{…}` sample with no name token).
pub(super) fn sample_name(line: &str) -> Option<Cow<'_, str>> {
    metric_and_labels(line.trim_start())
        .ok()
        .and_then(|(_, (name, _))| name)
}

/// The parsed metadata header of a single metric family.
///
/// Every field is optional: a family block may carry no descriptor lines at
/// all, in which case the caller derives the name from the first sample line
/// and treats the type as [`MetricType::Untyped`].
#[derive(Debug, Default, Clone, PartialEq)]
struct FamilyHeader<'a> {
    /// Family name, taken from the first descriptor line seen.
    name: Option<Cow<'a, str>>,
    help: Option<Cow<'a, str>>,
    unit: Option<Cow<'a, str>>,
    /// `None` when no `# TYPE` line was present.
    r#type: Option<MetricType>,
}

/// Parse the leading `# TYPE` / `# HELP` / `# UNIT` lines of an
/// already-separated metric family block.
///
/// Returns the assembled [`FamilyHeader`] and the remaining input, which
/// begins at the first non-descriptor line (the samples).
fn parse_header(input: &str) -> IResult<&str, FamilyHeader<'_>> {
    let (rest, descriptors) = many0(descriptor_line).parse(input)?;

    let mut header = FamilyHeader::default();
    for Descriptor { name, kind } in descriptors {
        // All descriptor lines in a well-formed family share one name; keep the
        // first and don't fail on a mismatch (permissive).
        if header.name.is_none() {
            header.name = Some(name);
        }
        match kind {
            DescKind::Type(t) => header.r#type = Some(t),
            DescKind::Help(h) => header.help = Some(h),
            DescKind::Unit(u) => header.unit = Some(u),
        }
    }
    Ok((rest, header))
}

// ---------------------------------------------------------------------------
// Header descriptor lines
// ---------------------------------------------------------------------------

enum DescKind<'a> {
    Type(MetricType),
    Help(Cow<'a, str>),
    Unit(Cow<'a, str>),
}

struct Descriptor<'a> {
    name: Cow<'a, str>,
    kind: DescKind<'a>,
}

/// One descriptor line: optional indentation, `#`, a keyword, its payload, and
/// the line ending.
fn descriptor_line(input: &str) -> IResult<&str, Descriptor<'_>> {
    delimited(
        (space0, char('#'), space1),
        alt((type_line, help_line, unit_line)),
        (space0, opt(line_ending)),
    )
    .parse(input)
}

fn type_line(input: &str) -> IResult<&str, Descriptor<'_>> {
    map(
        (tag("TYPE"), space1, metric_name, space1, type_value),
        |(_, _, name, _, ty)| Descriptor {
            name,
            kind: DescKind::Type(ty),
        },
    )
    .parse(input)
}

fn help_line(input: &str) -> IResult<&str, Descriptor<'_>> {
    map(
        (tag("HELP"), space1, metric_name, space0, rest_of_line),
        |(_, _, name, _, value)| Descriptor {
            name,
            kind: DescKind::Help(maybe_unescape(value)),
        },
    )
    .parse(input)
}

fn unit_line(input: &str) -> IResult<&str, Descriptor<'_>> {
    map(
        (tag("UNIT"), space1, metric_name, space0, rest_of_line),
        |(_, _, name, _, value)| Descriptor {
            name,
            kind: DescKind::Unit(maybe_unescape(value)),
        },
    )
    .parse(input)
}

/// The type keyword following `# TYPE <name>`, mapped onto [`MetricType`].
fn type_value(input: &str) -> IResult<&str, MetricType> {
    map(take_till1(is_space_or_eol), classify_type).parse(input)
}

fn classify_type(s: &str) -> MetricType {
    if s.eq_ignore_ascii_case("counter") {
        MetricType::Counter
    } else if s.eq_ignore_ascii_case("gauge") {
        MetricType::Gauge
    } else if s.eq_ignore_ascii_case("histogram") {
        MetricType::Histogram
    } else if s.eq_ignore_ascii_case("gaugehistogram") {
        MetricType::GaugeHistogram
    } else if s.eq_ignore_ascii_case("summary") {
        MetricType::Summary
    } else if s.eq_ignore_ascii_case("stateset") {
        MetricType::StateSet
    } else if s.eq_ignore_ascii_case("info") {
        MetricType::Info
    } else {
        // "unknown", "untyped", or anything we don't recognize.
        MetricType::Untyped
    }
}

// ---------------------------------------------------------------------------
// Sample lines
// ---------------------------------------------------------------------------

/// One parsed sample line, before it is folded into a [`Metric`] by type.
struct Sample<'a> {
    name: Cow<'a, str>,
    labels: Vec<LabelPair<'a>>,
    num: NumberToken,
    /// The raw trailing timestamp token, if any. Its unit (ms vs s) is resolved
    /// against the [`TextFormat`] when the metric is built, not here.
    timestamp: Option<NumberToken>,
    exemplar: Option<Exemplar<'a>>,
}

/// Split the post-header text into sample lines and parse each one.
///
/// Blank lines and bare comment lines are skipped; an `# EOF` marker ends the
/// block early.
fn parse_samples(rest: &str) -> Result<Vec<Sample<'_>>, ParseError> {
    let mut samples = Vec::new();
    for raw in rest.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(after_hash) = trimmed.strip_prefix('#') {
            if after_hash.trim() == "EOF" {
                break;
            }
            continue; // any other comment line
        }
        let (leftover, sample) =
            sample_line(line).map_err(|_| ParseError::InvalidLine(line.to_string()))?;
        if !leftover.is_empty() {
            return Err(ParseError::InvalidLine(line.to_string()));
        }
        samples.push(sample);
    }
    Ok(samples)
}

fn sample_line(input: &str) -> IResult<&str, Sample<'_>> {
    let (input, _) = space0.parse(input)?;
    let (input, (name, labels)) = metric_and_labels(input)?;
    let (input, _) = space1.parse(input)?;
    let (input, num) = number_token(input)?;
    let (input, timestamp) = opt(preceded(space1, number_token)).parse(input)?;
    let (input, exemplar) = opt(exemplar).parse(input)?;
    let (input, _) = space0.parse(input)?;
    Ok((
        input,
        Sample {
            name: name.unwrap_or(Cow::Borrowed("")),
            labels,
            num,
            timestamp,
            exemplar,
        },
    ))
}

/// Parse the metric name and its label set, in either order form:
///   * `name{labels}` / `name` (classic + OpenMetrics), or
///   * `{"name", labels}` (the Prometheus-3.0 UTF-8 form).
fn metric_and_labels(input: &str) -> IResult<&str, (Option<Cow<'_, str>>, Vec<LabelPair<'_>>)> {
    alt((braces_first_form, name_first_form)).parse(input)
}

fn name_first_form(input: &str) -> IResult<&str, (Option<Cow<'_, str>>, Vec<LabelPair<'_>>)> {
    map((metric_name, opt(labels_group)), |(name, labels)| {
        (Some(name), labels.unwrap_or_default())
    })
    .parse(input)
}

fn braces_first_form(input: &str) -> IResult<&str, (Option<Cow<'_, str>>, Vec<LabelPair<'_>>)> {
    let (input, _) = char('{').parse(input)?;
    let (input, _) = space0.parse(input)?;
    let (input, name) = opt(quoted_metric_name_in_braces).parse(input)?;
    let (input, _) = opt((space0, char(','))).parse(input)?;
    let (input, labels) = label_list(input)?;
    let (input, _) = (space0, char('}')).parse(input)?;
    Ok((input, (name, labels)))
}

/// A leading quoted metric name inside `{ … }` is distinguished from a quoted
/// *label* name by what follows it: a metric name is followed by `,` or `}`,
/// a label name by `=`.
fn quoted_metric_name_in_braces(input: &str) -> IResult<&str, Cow<'_, str>> {
    let (rest, name) = quoted_string(input)?;
    let (peeked, _) = space0.parse(rest)?;
    match peeked.chars().next() {
        Some(',') | Some('}') => Ok((rest, name)),
        _ => Err(Err::Error(NomError::new(input, ErrorKind::Tag))),
    }
}

fn labels_group(input: &str) -> IResult<&str, Vec<LabelPair<'_>>> {
    delimited(char('{'), label_list, (space0, char('}'))).parse(input)
}

/// Zero or more comma-separated labels, tolerating surrounding whitespace and a
/// trailing comma.
fn label_list(input: &str) -> IResult<&str, Vec<LabelPair<'_>>> {
    many0(delimited(space0, label, (space0, opt(char(','))))).parse(input)
}

fn label(input: &str) -> IResult<&str, LabelPair<'_>> {
    map(
        (label_name, space0, char('='), space0, quoted_string),
        |(name, _, _, _, value)| LabelPair { name, value },
    )
    .parse(input)
}

fn label_name(input: &str) -> IResult<&str, Cow<'_, str>> {
    alt((
        quoted_string,
        map(take_till1(is_label_name_end), Cow::Borrowed),
    ))
    .parse(input)
}

/// The exemplar trailer: ` # {labels} value [timestamp]`.
fn exemplar(input: &str) -> IResult<&str, Exemplar<'_>> {
    let (input, _) = (space0, char('#'), space0).parse(input)?;
    let (input, labels) = labels_group(input)?;
    let (input, _) = space1.parse(input)?;
    let (input, num) = number_token(input)?;
    let (input, ts) = opt(preceded(space1, number_token)).parse(input)?;
    Ok((
        input,
        Exemplar {
            label: labels,
            value: num.float,
            // Exemplar timestamps are always OpenMetrics seconds.
            timestamp: ts.map(|t| seconds_to_ts(t.float)),
        },
    ))
}

// ---------------------------------------------------------------------------
// Names, strings, numbers
// ---------------------------------------------------------------------------

/// A metric/label name: a double-quoted UTF-8 string or a bare run of
/// non-whitespace characters (stopping before `{`).
fn metric_name(input: &str) -> IResult<&str, Cow<'_, str>> {
    alt((
        quoted_string,
        map(
            take_till1(|c: char| is_space_or_eol(c) || c == '{'),
            Cow::Borrowed,
        ),
    ))
    .parse(input)
}

/// Everything from here to the end of the line (possibly empty).
fn rest_of_line(input: &str) -> IResult<&str, &str> {
    take_till(|c| c == '\r' || c == '\n').parse(input)
}

fn is_space_or_eol(c: char) -> bool {
    c == ' ' || c == '\t' || c == '\r' || c == '\n'
}

fn is_label_name_end(c: char) -> bool {
    is_space_or_eol(c) || matches!(c, '=' | ',' | '{' | '}' | '"')
}

/// Parse a double-quoted string, resolving `\\`, `\n` and `\"`. Borrows the
/// input when there are no escapes; allocates only when one is present.
fn quoted_string(input: &str) -> IResult<&str, Cow<'_, str>> {
    let (body, _) = char('"').parse(input)?;

    let mut has_escape = false;
    let mut close = None;
    let mut chars = body.char_indices();
    while let Some((idx, c)) = chars.next() {
        match c {
            '\\' => {
                has_escape = true;
                chars.next(); // skip the escaped char (UTF-8 safe)
            }
            '"' => {
                close = Some(idx);
                break;
            }
            _ => {}
        }
    }

    let Some(close) = close else {
        return Err(Err::Error(NomError::new(input, ErrorKind::TakeUntil)));
    };

    let inner = &body[..close];
    let after = &body[close + 1..]; // the closing quote is one byte
    let value = if has_escape {
        Cow::Owned(unescape(inner))
    } else {
        Cow::Borrowed(inner)
    };
    Ok((after, value))
}

/// Borrow `s` unchanged when it has no backslash; otherwise unescape into a
/// fresh `String`.
fn maybe_unescape(s: &str) -> Cow<'_, str> {
    if s.as_bytes().contains(&b'\\') {
        Cow::Owned(unescape(s))
    } else {
        Cow::Borrowed(s)
    }
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            // Lenient: an undefined escape keeps the following char verbatim.
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// A numeric token, retaining whether it was written as an integer so the
/// richer [`Number`](owned::Number) / [`UnsignedNumber`](owned::UnsignedNumber)
/// variants can be chosen.
#[derive(Debug, Clone, Copy)]
struct NumberToken {
    float: f64,
    int_value: Option<i64>,
    uint_value: Option<u64>,
}

impl NumberToken {
    /// Whether the token was written as a plain integer (no decimal point or
    /// exponent). Only such a token could be a classic-Prometheus millisecond
    /// timestamp, which is parsed as an `int64`.
    fn is_integer(&self) -> bool {
        self.int_value.is_some() || self.uint_value.is_some()
    }
}

/// Parse one whitespace-delimited numeric token, keeping its raw integer-ness.
/// Fails (recoverably) on a non-numeric token so callers can backtrack — e.g.
/// the optional timestamp slot stepping aside for an exemplar's `#`.
fn number_token(input: &str) -> IResult<&str, NumberToken> {
    let (rest, tok) = take_till1(is_space_or_eol).parse(input)?;
    match parse_number(tok) {
        Some(n) => Ok((rest, n)),
        None => Err(Err::Error(NomError::new(input, ErrorKind::Float))),
    }
}

fn parse_number(tok: &str) -> Option<NumberToken> {
    let int_value = tok.parse::<i64>().ok();
    let uint_value = tok.parse::<u64>().ok();
    let float = if let Some(i) = int_value {
        i as f64
    } else if let Some(u) = uint_value {
        u as f64
    } else {
        parse_float(tok)?
    };
    Some(NumberToken {
        float,
        int_value,
        uint_value,
    })
}

/// `f64` parse that already accepts `nan`/`inf`/`infinity` case-insensitively
/// with an optional sign — exactly the OpenMetrics special values.
fn parse_float(tok: &str) -> Option<f64> {
    tok.parse::<f64>().ok()
}

fn number_from(t: &NumberToken) -> owned::Number {
    match t.int_value {
        Some(i) => owned::Number::Int(i),
        None => owned::Number::Float(t.float),
    }
}

fn unsigned_from(t: &NumberToken) -> owned::UnsignedNumber {
    match t.uint_value {
        Some(u) => owned::UnsignedNumber::Uint(u),
        None => owned::UnsignedNumber::Float(t.float),
    }
}

fn u64_from(t: &NumberToken) -> u64 {
    t.uint_value.unwrap_or(t.float as u64)
}

// ---------------------------------------------------------------------------
// Timestamps
// ---------------------------------------------------------------------------

#[cfg(not(feature = "chrono"))]
fn make_ts(seconds: i64, nanos: u32) -> Ts {
    owned::Timestamp {
        seconds,
        nanos: nanos as i32,
    }
}

#[cfg(feature = "chrono")]
fn make_ts(seconds: i64, nanos: u32) -> Ts {
    use chrono::TimeZone;
    chrono::Utc
        .timestamp_opt(seconds, nanos)
        .single()
        .unwrap_or_else(|| chrono::Utc.timestamp_opt(0, 0).single().unwrap())
}

/// Resolve a raw trailing-timestamp token to a [`Ts`] using the dialect's unit.
fn convert_ts(format: TextFormat, t: &NumberToken) -> Ts {
    match format {
        TextFormat::OpenMetrics => seconds_to_ts(t.float),
        // Classic Prometheus trailing timestamps are integer milliseconds.
        TextFormat::Prometheus => seconds_to_ts(t.float / 1000.0),
        TextFormat::Guess => {
            if !t.is_integer() {
                // A decimal point or exponent rules out a classic int64
                // millisecond timestamp, so this is unambiguously OpenMetrics
                // seconds — no need to guess.
                seconds_to_ts(t.float)
            } else if let Some(now) = now_unix_seconds() {
                let as_seconds = t.float; // OpenMetrics reading
                let as_millis = t.float / 1000.0; // classic Prometheus reading
                // Keep whichever lands closer to "now"; a tie favours seconds.
                if (as_millis - now).abs() < (as_seconds - now).abs() {
                    seconds_to_ts(as_millis)
                } else {
                    seconds_to_ts(as_seconds)
                }
            } else {
                // No usable clock to compare against: fall back to OpenMetrics.
                seconds_to_ts(t.float)
            }
        }
    }
}

/// Current wall-clock time as fractional Unix seconds, or `None` if the system
/// clock is set before the epoch.
fn now_unix_seconds() -> Option<f64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs_f64())
}

fn seconds_to_ts(secs: f64) -> Ts {
    if !secs.is_finite() {
        return make_ts(0, 0);
    }
    let whole = secs.floor();
    let mut seconds = whole as i64;
    let mut nanos = ((secs - whole) * 1_000_000_000.0).round() as i64;
    if nanos >= 1_000_000_000 {
        seconds = seconds.saturating_add(1);
        nanos -= 1_000_000_000;
    }
    if nanos < 0 {
        nanos = 0;
    }
    make_ts(seconds, nanos as u32)
}

// ---------------------------------------------------------------------------
// Folding samples into Metrics, by family type
// ---------------------------------------------------------------------------

/// The role a sample line plays within its family, derived from its name suffix
/// relative to the family base name.
#[derive(Clone, Copy, PartialEq)]
enum Role {
    Bare,
    Total,
    Sum,
    Count,
    Bucket,
    Created,
    GCount,
    GSum,
    Other,
}

fn role(name: &str, base: &str) -> Role {
    if name == base {
        return Role::Bare;
    }
    match name.strip_prefix(base) {
        Some("_total") => Role::Total,
        Some("_sum") => Role::Sum,
        Some("_count") => Role::Count,
        Some("_bucket") => Role::Bucket,
        Some("_created") => Role::Created,
        Some("_gcount") => Role::GCount,
        Some("_gsum") => Role::GSum,
        _ => Role::Other,
    }
}

fn assemble<'a>(
    format: TextFormat,
    ty: MetricType,
    base: &str,
    samples: Vec<Sample<'a>>,
) -> Vec<Metric<'a>> {
    match ty {
        // Native/hybrid never arise from text; treat them as plain scalars.
        MetricType::Gauge
        | MetricType::Untyped
        | MetricType::NativeHistogram
        | MetricType::HybridHistogram => scalar_metrics(format, ty, samples),
        MetricType::Info => info_metrics(format, samples),
        MetricType::Counter
        | MetricType::Summary
        | MetricType::Histogram
        | MetricType::GaugeHistogram
        | MetricType::StateSet => grouped_metrics(format, ty, base, samples),
    }
}

/// Gauge / unknown: one metric per line, no grouping.
fn scalar_metrics(
    format: TextFormat,
    ty: MetricType,
    samples: Vec<Sample>,
) -> Vec<Metric> {
    samples
        .into_iter()
        .map(|s| {
            let n = number_from(&s.num);
            let value = if ty == MetricType::Gauge {
                MetricValue::Gauge(n)
            } else {
                MetricValue::Untyped(n)
            };
            Metric {
                label: s.labels,
                value,
                timestamp: s.timestamp.as_ref().map(|t| convert_ts(format, t)),
            }
        })
        .collect()
}

/// Info: one metric per `_info` line; the line's labels become the info
/// payload and the metric carries no identifying labels of its own.
fn info_metrics(format: TextFormat, samples: Vec<Sample>) -> Vec<Metric> {
    samples
        .into_iter()
        .map(|s| Metric {
            label: Vec::new(),
            value: MetricValue::Info(Info { labels: s.labels }),
            timestamp: s.timestamp.as_ref().map(|t| convert_ts(format, t)),
        })
        .collect()
}

/// Per-metric accumulator while folding the many sample lines of a histogram /
/// summary / counter / stateset into a single [`Metric`].
struct Acc<'a> {
    labels: Vec<LabelPair<'a>>,
    timestamp: Option<Ts>,
    value: Option<NumberToken>,
    exemplar: Option<Exemplar<'a>>,
    sum: Option<NumberToken>,
    count: Option<NumberToken>,
    created: Option<Ts>,
    buckets: Vec<BucketAcc<'a>>,
    quantiles: Vec<owned::Quantile>,
    states: Vec<State<'a>>,
}

struct BucketAcc<'a> {
    upper: f64,
    count: NumberToken,
    exemplar: Option<Exemplar<'a>>,
}

impl<'a> Acc<'a> {
    fn new(labels: Vec<LabelPair<'a>>) -> Self {
        Acc {
            labels,
            timestamp: None,
            value: None,
            exemplar: None,
            sum: None,
            count: None,
            created: None,
            buckets: Vec::new(),
            quantiles: Vec::new(),
            states: Vec::new(),
        }
    }
}

fn grouped_metrics<'a>(
    format: TextFormat,
    ty: MetricType,
    base: &str,
    samples: Vec<Sample<'a>>,
) -> Vec<Metric<'a>> {
    // The label whose presence distinguishes the sub-samples of one metric and
    // which is therefore stripped before computing the grouping key.
    let differentiator: Option<&str> = match ty {
        MetricType::Histogram | MetricType::GaugeHistogram => Some("le"),
        MetricType::Summary => Some("quantile"),
        MetricType::StateSet => Some(base),
        _ => None,
    };

    let mut order: Vec<Acc<'a>> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();

    for mut s in samples {
        let r = role(&s.name, base);

        let removed = match differentiator {
            Some(dn) => match s.labels.iter().position(|l| l.name.as_ref() == dn) {
                Some(pos) => Some(s.labels.remove(pos).value),
                None => None,
            },
            None => None,
        };

        let key = group_key(&s.labels);
        let (i, is_new) = group_index(&mut index, order.len(), key);
        if is_new {
            order.push(Acc::new(std::mem::take(&mut s.labels)));
        }
        let acc = &mut order[i];
        if acc.timestamp.is_none() {
            acc.timestamp = s.timestamp.as_ref().map(|t| convert_ts(format, t));
        }

        match (ty, r) {
            (MetricType::Counter, Role::Bare | Role::Total) => {
                acc.value = Some(s.num);
                if acc.exemplar.is_none() {
                    acc.exemplar = s.exemplar.take();
                }
            }
            (MetricType::Counter, Role::Created) => acc.created = Some(seconds_to_ts(s.num.float)),

            (MetricType::Summary, Role::Bare) => {
                if let Some(q) = removed.as_deref().and_then(parse_float) {
                    acc.quantiles.push(owned::Quantile {
                        quantile: q,
                        value: s.num.float,
                    });
                }
            }
            (MetricType::Summary, Role::Sum) => acc.sum = Some(s.num),
            (MetricType::Summary, Role::Count) => acc.count = Some(s.num),
            (MetricType::Summary, Role::Created) => acc.created = Some(seconds_to_ts(s.num.float)),

            (MetricType::Histogram | MetricType::GaugeHistogram, Role::Bucket) => {
                if let Some(upper) = removed.as_deref().and_then(parse_float) {
                    acc.buckets.push(BucketAcc {
                        upper,
                        count: s.num,
                        exemplar: s.exemplar.take(),
                    });
                }
            }
            (MetricType::Histogram, Role::Sum) => acc.sum = Some(s.num),
            (MetricType::Histogram, Role::Count) => acc.count = Some(s.num),
            (MetricType::GaugeHistogram, Role::GSum) => acc.sum = Some(s.num),
            (MetricType::GaugeHistogram, Role::GCount) => acc.count = Some(s.num),
            (MetricType::Histogram | MetricType::GaugeHistogram, Role::Created) => {
                acc.created = Some(seconds_to_ts(s.num.float))
            }

            (MetricType::StateSet, _) => acc.states.push(State {
                name: removed.unwrap_or(Cow::Borrowed("")),
                enabled: s.num.float != 0.0,
            }),

            _ => {} // unrecognized role for this family type — ignore
        }
    }

    order
        .into_iter()
        .map(move |acc| {
            let timestamp = acc.timestamp;
            let value = match ty {
                MetricType::Counter => MetricValue::Counter(Counter {
                    value: acc
                        .value
                        .as_ref()
                        .map(unsigned_from)
                        .unwrap_or(owned::UnsignedNumber::Uint(0)),
                    exemplar: acc.exemplar,
                    created_timestamp: acc.created,
                }),
                MetricType::Summary => MetricValue::Summary(owned::Summary {
                    sample_count: acc.count.as_ref().map(u64_from),
                    sample_sum: acc.sum.as_ref().map(number_from),
                    quantile: acc.quantiles,
                    created_timestamp: acc.created,
                }),
                MetricType::Histogram => MetricValue::Histogram(build_histogram(
                    acc.sum,
                    acc.count,
                    acc.buckets,
                    acc.created,
                )),
                MetricType::GaugeHistogram => MetricValue::GaugeHistogram(build_histogram(
                    acc.sum,
                    acc.count,
                    acc.buckets,
                    acc.created,
                )),
                MetricType::StateSet => MetricValue::StateSet(StateSet { states: acc.states }),
                _ => unreachable!("grouped_metrics only handles grouped family types"),
            };
            Metric {
                label: acc.labels,
                value,
                timestamp,
            }
        })
        .collect()
}

/// Build a classic [`Histogram`] from the accumulated parts. The whole
/// histogram is integer-valued unless any count carried a fractional value.
fn build_histogram(
    sum: Option<NumberToken>,
    count: Option<NumberToken>,
    buckets: Vec<BucketAcc>,
    created: Option<Ts>,
) -> Histogram {
    let all_int = buckets.iter().all(|b| b.count.uint_value.is_some())
        && count.is_none_or(|c| c.uint_value.is_some());

    let counts = if all_int {
        BucketCount::Int {
            sample_count: count.as_ref().map(u64_from),
            buckets: buckets
                .into_iter()
                .map(|b| BucketInt {
                    cumulative_count: u64_from(&b.count),
                    upper_bound: b.upper,
                    exemplar: b.exemplar,
                })
                .collect(),
        }
    } else {
        BucketCount::Float {
            sample_count: count.map(|c| c.float),
            buckets: buckets
                .into_iter()
                .map(|b| BucketFloat {
                    cumulative_count: b.count.float,
                    upper_bound: b.upper,
                    exemplar: b.exemplar,
                })
                .collect(),
        }
    };

    Histogram {
        sample_sum: sum.as_ref().map(number_from),
        counts,
        created_timestamp: created,
    }
}

/// A stable, order-independent key for a label set, used to group the sample
/// lines that belong to the same metric.
fn group_key(labels: &[LabelPair<'_>]) -> String {
    let mut pairs: Vec<(&str, &str)> = labels
        .iter()
        .map(|l| (l.name.as_ref(), l.value.as_ref()))
        .collect();
    pairs.sort_unstable();
    let mut key = String::new();
    for (n, v) in pairs {
        key.push_str(n);
        key.push('\u{1}');
        key.push_str(v);
        key.push('\u{2}');
    }
    key
}

fn group_index(index: &mut HashMap<String, usize>, len: usize, key: String) -> (usize, bool) {
    match index.get(&key) {
        Some(&i) => (i, false),
        None => {
            index.insert(key, len);
            (len, true)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- header --------------------------------------------------------

    #[test]
    fn parses_canonical_order() {
        let input = "# TYPE foo_seconds histogram\n\
                     # UNIT foo_seconds seconds\n\
                     # HELP foo_seconds A histogram.\n\
                     foo_seconds_count 5\n";
        let (rest, h) = parse_header(input).unwrap();
        assert_eq!(h.name.as_deref(), Some("foo_seconds"));
        assert_eq!(h.r#type, Some(MetricType::Histogram));
        assert_eq!(h.unit.as_deref(), Some("seconds"));
        assert_eq!(h.help.as_deref(), Some("A histogram."));
        assert_eq!(rest, "foo_seconds_count 5\n");
    }

    #[test]
    fn order_independent_help_before_type() {
        let input = "# HELP x docs\n# TYPE x counter\nx_total 1\n";
        let (rest, h) = parse_header(input).unwrap();
        assert_eq!(h.help.as_deref(), Some("docs"));
        assert_eq!(h.r#type, Some(MetricType::Counter));
        assert_eq!(rest, "x_total 1\n");
    }

    #[test]
    fn missing_type_is_none() {
        let input = "# HELP x docs\nx 1\n";
        let (_rest, h) = parse_header(input).unwrap();
        assert_eq!(h.r#type, None);
        assert_eq!(h.help.as_deref(), Some("docs"));
    }

    #[test]
    fn unknown_untyped_and_garbage_map_to_untyped() {
        for kw in ["unknown", "untyped", "UNKNOWN", "weirdtype"] {
            let input = format!("# TYPE x {kw}\n");
            let (_r, h) = parse_header(&input).unwrap();
            assert_eq!(h.r#type, Some(MetricType::Untyped), "kw={kw}");
        }
    }

    #[test]
    fn escaped_help_allocates_and_resolves() {
        // Wire: line\none\\two \"q\"  ->  line<LF>one\two "q"
        let input = "# HELP x line\\none\\\\two \\\"q\\\"\n";
        let (_r, h) = parse_header(input).unwrap();
        assert_eq!(h.help.as_deref(), Some("line\none\\two \"q\""));
        assert!(matches!(h.help, Some(Cow::Owned(_))));
    }

    // ---- helpers -------------------------------------------------------

    fn one<'a, 'b>(metrics: &'a [Metric<'b>]) -> &'a Metric<'b> {
        assert_eq!(metrics.len(), 1, "expected exactly one metric");
        &metrics[0]
    }

    fn label<'a>(m: &'a Metric<'_>, name: &str) -> Option<&'a str> {
        m.label
            .iter()
            .find(|l| l.name == name)
            .map(|l| l.value.as_ref())
    }

    // ---- full family parsing ------------------------------------------

    #[test]
    fn gauge_int_vs_float() {
        let text = "# TYPE temp gauge\n\
                    # HELP temp Temperature\n\
                    temp{room=\"kitchen\"} 21.5\n\
                    temp{room=\"bath\"} 19\n";
        let fam = parse_family(text, TextFormat::OpenMetrics).unwrap();
        assert_eq!(fam.name, "temp");
        assert_eq!(fam.r#type, MetricType::Gauge);
        assert_eq!(fam.help.as_deref(), Some("Temperature"));
        assert_eq!(fam.metric.len(), 2);

        assert_eq!(label(&fam.metric[0], "room"), Some("kitchen"));
        assert!(matches!(
            fam.metric[0].value,
            MetricValue::Gauge(owned::Number::Float(v)) if v == 21.5
        ));
        // "19" has no decimal point -> integer.
        assert!(matches!(
            fam.metric[1].value,
            MetricValue::Gauge(owned::Number::Int(19))
        ));
    }

    #[test]
    fn untyped_without_header_derives_name() {
        let fam = parse_family("foo_bar 42\n", TextFormat::Prometheus).unwrap();
        assert_eq!(fam.name, "foo_bar");
        assert_eq!(fam.r#type, MetricType::Untyped);
        assert!(matches!(
            one(&fam.metric).value,
            MetricValue::Untyped(owned::Number::Int(42))
        ));
    }

    #[test]
    fn counter_classic_and_openmetrics_with_exemplar_and_created() {
        let text = "# TYPE http_requests counter\n\
                    http_requests_total{code=\"200\"} 1027 # {trace_id=\"abc\"} 1.0\n\
                    http_requests_created{code=\"200\"} 1605.5\n";
        let fam = parse_family(text, TextFormat::OpenMetrics).unwrap();
        let m = one(&fam.metric);
        assert_eq!(label(m, "code"), Some("200"));
        let MetricValue::Counter(c) = &m.value else {
            panic!("expected counter");
        };
        assert!(matches!(c.value, owned::UnsignedNumber::Uint(1027)));
        let ex = c.exemplar.as_ref().expect("exemplar");
        assert_eq!(ex.value, 1.0);
        assert_eq!(ex.label[0].name, "trace_id");
        assert_eq!(ex.label[0].value, "abc");
        assert!(c.created_timestamp.is_some());
    }

    #[test]
    fn classic_counter_bare_name() {
        let fam = parse_family("# TYPE foo counter\nfoo 5\n", TextFormat::Prometheus).unwrap();
        let MetricValue::Counter(c) = &one(&fam.metric).value else {
            panic!("expected counter");
        };
        assert!(matches!(c.value, owned::UnsignedNumber::Uint(5)));
    }

    #[test]
    fn histogram_buckets_sum_count() {
        let text = "# TYPE rt histogram\n\
                    rt_bucket{le=\"0.1\"} 1\n\
                    rt_bucket{le=\"+Inf\"} 3\n\
                    rt_sum 0.35\n\
                    rt_count 3\n";
        let fam = parse_family(text, TextFormat::OpenMetrics).unwrap();
        let MetricValue::Histogram(h) = &one(&fam.metric).value else {
            panic!("expected histogram");
        };
        assert_eq!(h.sample_sum, Some(owned::Number::Float(0.35)));
        let BucketCount::Int {
            sample_count,
            buckets,
        } = &h.counts
        else {
            panic!("expected integer buckets");
        };
        assert_eq!(*sample_count, Some(3));
        assert_eq!(buckets.len(), 2);
        assert_eq!(buckets[0].upper_bound, 0.1);
        assert_eq!(buckets[0].cumulative_count, 1);
        assert!(buckets[1].upper_bound.is_infinite());
        assert_eq!(buckets[1].cumulative_count, 3);
    }

    #[test]
    fn histogram_groups_by_outer_labels() {
        let text = "# TYPE rt histogram\n\
                    rt_bucket{path=\"/a\",le=\"0.1\"} 1\n\
                    rt_bucket{path=\"/a\",le=\"+Inf\"} 2\n\
                    rt_count{path=\"/a\"} 2\n\
                    rt_bucket{path=\"/b\",le=\"+Inf\"} 5\n\
                    rt_count{path=\"/b\"} 5\n";
        let fam = parse_family(text, TextFormat::OpenMetrics).unwrap();
        assert_eq!(fam.metric.len(), 2);
        assert_eq!(label(&fam.metric[0], "path"), Some("/a"));
        assert_eq!(label(&fam.metric[1], "path"), Some("/b"));
        // the `le` label must not survive on the grouped metric
        assert_eq!(label(&fam.metric[0], "le"), None);
    }

    #[test]
    fn float_histogram_detected_from_fractional_count() {
        let text = "# TYPE fh histogram\nfh_bucket{le=\"+Inf\"} 2.5\nfh_count 2.5\n";
        let fam = parse_family(text, TextFormat::OpenMetrics).unwrap();
        let MetricValue::Histogram(h) = &one(&fam.metric).value else {
            panic!("expected histogram");
        };
        let BucketCount::Float {
            sample_count,
            buckets,
        } = &h.counts
        else {
            panic!("expected float buckets");
        };
        assert_eq!(*sample_count, Some(2.5));
        assert_eq!(buckets[0].cumulative_count, 2.5);
    }

    #[test]
    fn gauge_histogram_uses_gsum_gcount() {
        let text = "# TYPE gh gaugehistogram\n\
                    gh_bucket{le=\"1\"} 2\n\
                    gh_bucket{le=\"+Inf\"} 5\n\
                    gh_gsum 8.0\n\
                    gh_gcount 5\n";
        let fam = parse_family(text, TextFormat::OpenMetrics).unwrap();
        let MetricValue::GaugeHistogram(h) = &one(&fam.metric).value else {
            panic!("expected gauge histogram");
        };
        assert_eq!(h.sample_sum, Some(owned::Number::Float(8.0)));
        let BucketCount::Int { sample_count, .. } = &h.counts else {
            panic!("expected integer buckets");
        };
        assert_eq!(*sample_count, Some(5));
    }

    #[test]
    fn summary_quantiles_sum_count() {
        let text = "# TYPE lat summary\n\
                    lat{quantile=\"0.5\"} 0.2\n\
                    lat{quantile=\"0.9\"} 0.5\n\
                    lat_sum 12.3\n\
                    lat_count 100\n";
        let fam = parse_family(text, TextFormat::OpenMetrics).unwrap();
        let MetricValue::Summary(s) = &one(&fam.metric).value else {
            panic!("expected summary");
        };
        assert_eq!(s.sample_count, Some(100));
        assert_eq!(s.sample_sum, Some(owned::Number::Float(12.3)));
        assert_eq!(s.quantile.len(), 2);
        assert_eq!(s.quantile[0].quantile, 0.5);
        assert_eq!(s.quantile[0].value, 0.2);
        assert_eq!(s.quantile[1].quantile, 0.9);
    }

    #[test]
    fn stateset_collects_states() {
        let text = "# TYPE myset stateset\nmyset{myset=\"a\"} 1\nmyset{myset=\"b\"} 0\n";
        let fam = parse_family(text, TextFormat::OpenMetrics).unwrap();
        let m = one(&fam.metric);
        assert!(m.label.is_empty());
        let MetricValue::StateSet(ss) = &m.value else {
            panic!("expected stateset");
        };
        assert_eq!(ss.states.len(), 2);
        assert_eq!(ss.states[0].name, "a");
        assert!(ss.states[0].enabled);
        assert_eq!(ss.states[1].name, "b");
        assert!(!ss.states[1].enabled);
    }

    #[test]
    fn stateset_groups_by_identity_labels() {
        let text = "# TYPE feature stateset\n\
                    feature{env=\"prod\",feature=\"x\"} 1\n\
                    feature{env=\"prod\",feature=\"y\"} 0\n";
        let fam = parse_family(text, TextFormat::OpenMetrics).unwrap();
        let m = one(&fam.metric);
        assert_eq!(label(m, "env"), Some("prod"));
        let MetricValue::StateSet(ss) = &m.value else {
            panic!("expected stateset");
        };
        assert_eq!(ss.states.len(), 2);
    }

    #[test]
    fn info_labels_become_payload() {
        let text = "# TYPE build info\nbuild_info{version=\"1.2.3\",branch=\"main\"} 1\n";
        let fam = parse_family(text, TextFormat::OpenMetrics).unwrap();
        let m = one(&fam.metric);
        assert!(m.label.is_empty());
        let MetricValue::Info(info) = &m.value else {
            panic!("expected info");
        };
        assert_eq!(info.labels.len(), 2);
        assert_eq!(info.labels[0].name, "version");
        assert_eq!(info.labels[0].value, "1.2.3");
    }

    #[test]
    fn utf8_name_in_braces_form() {
        let text = "# TYPE \"my.gauge\" gauge\n{\"my.gauge\",room=\"k\"} 3.5\n";
        let fam = parse_family(text, TextFormat::OpenMetrics).unwrap();
        assert_eq!(fam.name, "my.gauge");
        let m = one(&fam.metric);
        assert_eq!(label(m, "room"), Some("k"));
        assert!(matches!(
            m.value,
            MetricValue::Gauge(owned::Number::Float(v)) if v == 3.5
        ));
    }

    #[test]
    fn label_value_escapes_and_eof() {
        let text = "# TYPE m gauge\nm{path=\"C:\\\\dir\",msg=\"a\\nb\"} 1\n# EOF\nignored 9\n";
        let fam = parse_family(text, TextFormat::OpenMetrics).unwrap();
        let m = one(&fam.metric);
        assert_eq!(label(m, "path"), Some("C:\\dir"));
        assert_eq!(label(m, "msg"), Some("a\nb"));
    }

    #[test]
    fn no_labels_and_trailing_timestamp() {
        let fam = parse_family(
            "# TYPE g gauge\ng 12.47 1604676851\n",
            TextFormat::OpenMetrics,
        )
        .unwrap();
        let m = one(&fam.metric);
        assert!(m.label.is_empty());
        assert!(m.timestamp.is_some());
    }

    #[test]
    fn timestamp_units_differ_by_format() {
        // 1604676851000 ms (classic) and 1604676851 s (OpenMetrics) are the
        // same instant — so the two parses must agree.
        let prom = parse_family(
            "# TYPE g gauge\ng 1 1604676851000\n",
            TextFormat::Prometheus,
        )
        .unwrap();
        let om = parse_family("# TYPE g gauge\ng 1 1604676851\n", TextFormat::OpenMetrics).unwrap();
        assert!(prom.metric[0].timestamp.is_some());
        assert_eq!(prom.metric[0].timestamp, om.metric[0].timestamp);
    }

    #[test]
    fn guess_resolves_each_unit_by_proximity_to_now() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let now_s = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // The same recent instant written as OpenMetrics seconds and as classic
        // milliseconds. Guess should read each in its own unit, so both parses
        // land on the same timestamp.
        let secs_text = format!("# TYPE g gauge\ng 1 {now_s}\n");
        let millis_text = format!("# TYPE g gauge\ng 1 {}\n", now_s * 1000);

        let secs = parse_family(&secs_text, TextFormat::Guess).unwrap();
        let millis = parse_family(&millis_text, TextFormat::Guess).unwrap();
        assert!(secs.metric[0].timestamp.is_some());
        assert_eq!(secs.metric[0].timestamp, millis.metric[0].timestamp);

        // And it agrees with the explicit seconds dialect.
        let explicit = parse_family(&secs_text, TextFormat::OpenMetrics).unwrap();
        assert_eq!(secs.metric[0].timestamp, explicit.metric[0].timestamp);
    }

    #[test]
    fn guess_reads_fractional_timestamp_as_seconds() {
        // 1700000000000.5 read as milliseconds would land near "now" and so win
        // the proximity heuristic — but the decimal point proves it can't be a
        // classic int64 ms timestamp, so Guess must read it as seconds instead.
        let text = "# TYPE g gauge\ng 1 1700000000000.5\n";
        let guessed = parse_family(text, TextFormat::Guess).unwrap();
        let as_seconds = parse_family(text, TextFormat::OpenMetrics).unwrap();
        let as_millis = parse_family(text, TextFormat::Prometheus).unwrap();

        assert_eq!(guessed.metric[0].timestamp, as_seconds.metric[0].timestamp);
        assert_ne!(guessed.metric[0].timestamp, as_millis.metric[0].timestamp);
    }

    #[test]
    fn malformed_value_is_an_error() {
        let err = parse_family("# TYPE g gauge\ng abc\n", TextFormat::OpenMetrics).unwrap_err();
        assert!(matches!(err, ParseError::InvalidLine(_)));
    }

    #[test]
    fn blank_lines_are_ignored() {
        let fam = parse_family("# TYPE g gauge\n\ng 1\n\n", TextFormat::OpenMetrics).unwrap();
        assert_eq!(fam.metric.len(), 1);
    }
}
