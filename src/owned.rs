//! The owned, allocation-backed data model.
//!
//! These are the types you get after calling
//! [`into_owned`](crate::borrowed::MetricFamily::into_owned) on a parsed family,
//! and what the [`Decoder`](crate::Decoder) and the async `Client` hand
//! back directly. Every string is a `String` and every collection is owned, so a
//! value here is independent of the buffer it was parsed from — free to store,
//! move between threads, or return from a function.
//!
//! The shapes mirror the borrowed model in [`crate::borrowed`] one-for-one;
//! [`MetricFamily`] is the root. The only structural subtlety is the histogram
//! family — see [`MetricValue`] for how classic, gauge, native, and hybrid
//! histograms are distinguished.

/// A point in time as raw `seconds` + `nanos` since the Unix epoch.
///
/// This is the timestamp representation when the `chrono` feature is **off**;
/// with the (default) `chrono` feature, timestamp fields are
/// [`chrono::DateTime<Utc>`](chrono::DateTime) instead.
#[cfg(not(feature = "chrono"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timestamp {
    pub seconds: i64,
    pub nanos: i32,
}

/// A numeric value that kept the integer-vs-float distinction it was written
/// with: `5` decodes as [`Int`](Number::Int), `5.0` as [`Float`](Number::Float).
///
/// Use [`as_f64`](Number::as_f64) when you only need the magnitude.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Number {
    Int(i64),
    Float(f64),
}

impl Number {
    pub fn as_f64(self) -> f64 {
        match self {
            Number::Int(i) => i as f64,
            Number::Float(f) => f,
        }
    }
}

/// Like [`Number`] but for values that cannot be negative (counter totals,
/// sample counts): an unsigned [`Uint`](UnsignedNumber::Uint) or a
/// [`Float`](UnsignedNumber::Float).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnsignedNumber {
    Uint(u64),
    Float(f64),
}

impl UnsignedNumber {
    pub fn as_f64(self) -> f64 {
        match self {
            UnsignedNumber::Uint(u) => u as f64,
            UnsignedNumber::Float(f) => f,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Counter {
    pub value: UnsignedNumber,
    pub exemplar: Option<Exemplar>,
    #[cfg(not(feature = "chrono"))]
    pub created_timestamp: Option<Timestamp>,
    #[cfg(feature = "chrono")]
    pub created_timestamp: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone)]
pub struct Summary {
    pub sample_count: Option<u64>,
    pub sample_sum: Option<Number>,
    pub quantile: Vec<Quantile>,
    #[cfg(not(feature = "chrono"))]
    pub created_timestamp: Option<Timestamp>,
    #[cfg(feature = "chrono")]
    pub created_timestamp: Option<chrono::DateTime<chrono::Utc>>,
}

/// A classic (explicit-bucket) histogram, also used for gauge histograms. Its
/// buckets and sample count live in [`counts`](Histogram::counts), which is
/// integer- or float-valued as a unit.
#[derive(Debug, Clone)]
pub struct Histogram {
    pub sample_sum: Option<Number>,
    pub counts: BucketCount,
    #[cfg(not(feature = "chrono"))]
    pub created_timestamp: Option<Timestamp>,
    #[cfg(feature = "chrono")]
    pub created_timestamp: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone)]
pub struct BucketSpan {
    pub offset: i32,
    pub length: u32,
}

/// A Prometheus native (exponential, sparse-bucket) histogram. These are
/// protobuf-only — the text format cannot express them. Bucket layout is given
/// by `schema` plus the spans/deltas in [`counts`](NativeHistogram::counts).
#[derive(Debug, Clone)]
pub struct NativeHistogram {
    pub schema: i32,
    pub zero_threshold: f64,
    pub sample_sum: Option<Number>,
    pub counts: NativeCounts,
    pub exemplars: Vec<Exemplar>,
    #[cfg(not(feature = "chrono"))]
    pub created_timestamp: Option<Timestamp>,
    #[cfg(feature = "chrono")]
    pub created_timestamp: Option<chrono::DateTime<chrono::Utc>>,
}

/// The bucket counts of a [`NativeHistogram`], either integer-valued (deltas)
/// or float-valued (absolute counts). The two are mutually exclusive on the wire.
#[derive(Debug, Clone)]
pub enum NativeCounts {
    Int {
        sample_count: Option<u64>,
        zero_count: u64,
        positive_spans: Vec<BucketSpan>,
        positive_deltas: Vec<i64>,
        negative_spans: Vec<BucketSpan>,
        negative_deltas: Vec<i64>,
    },
    Float {
        sample_count: Option<f64>,
        zero_count: f64,
        positive_spans: Vec<BucketSpan>,
        positive_counts: Vec<f64>,
        negative_spans: Vec<BucketSpan>,
        negative_counts: Vec<f64>,
    },
}

#[derive(Debug, Clone)]
pub struct Info {
    pub labels: Vec<LabelPair>,
}

#[derive(Debug, Clone)]
pub struct State {
    pub name: String,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct StateSet {
    pub states: Vec<State>,
}

/// The actual measurement carried by one [`Metric`], tagged by kind.
///
/// All families of a given [`MetricType`] produce the matching variant, with one
/// nuance around histograms: a protobuf histogram may carry classic buckets, a
/// native (exponential) layout, or *both* at once, so the type splits into
/// [`Histogram`](MetricValue::Histogram),
/// [`NativeHistogram`](MetricValue::NativeHistogram), and
/// [`HybridHistogram`](MetricValue::HybridHistogram). Native and hybrid variants
/// only ever come from the protobuf format; text parsing never produces them.
#[derive(Debug, Clone)]
pub enum MetricValue {
    Counter(Counter),
    Gauge(Number),
    Summary(Summary),
    Untyped(Number),
    /// A classic explicit-bucket histogram.
    Histogram(Histogram),
    /// A gauge histogram (buckets that can go down as well as up).
    GaugeHistogram(Histogram),
    /// A native (exponential) histogram — protobuf only.
    NativeHistogram(NativeHistogram),
    /// A single histogram carrying both classic buckets and a native layout —
    /// protobuf only.
    HybridHistogram {
        classic: Histogram,
        native: NativeHistogram,
    },
    StateSet(StateSet),
    Info(Info),
}

/// The declared type of a [`MetricFamily`], from its `# TYPE` line (text) or the
/// `type` field (protobuf). Determines which [`MetricValue`] variant each metric
/// in the family carries. An unrecognized or absent type becomes
/// [`Untyped`](MetricType::Untyped).
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum MetricType {
    Counter,
    Gauge,
    Summary,
    Untyped,
    Histogram,
    GaugeHistogram,
    NativeHistogram,
    HybridHistogram,
    StateSet,
    Info,
}

/// The buckets and sample count of a classic [`Histogram`]. The whole histogram
/// is integer-valued unless any bucket or the count carried a fractional value,
/// in which case it is [`Float`](BucketCount::Float).
#[derive(Debug, Clone)]
pub enum BucketCount {
    Int {
        sample_count: Option<u64>,
        buckets: Vec<BucketInt>,
    },
    Float {
        sample_count: Option<f64>,
        buckets: Vec<BucketFloat>,
    },
}

#[derive(Debug, Clone)]
pub struct BucketFloat {
    pub cumulative_count: f64,
    pub upper_bound: f64,
    pub exemplar: Option<Exemplar>,
}

#[derive(Debug, Clone)]
pub struct BucketInt {
    pub cumulative_count: u64,
    pub upper_bound: f64,
    pub exemplar: Option<Exemplar>,
}

#[derive(Debug, Clone)]
pub struct Exemplar {
    pub label: Vec<LabelPair>,
    pub value: f64,
    #[cfg(not(feature = "chrono"))]
    pub timestamp: Option<Timestamp>,
    #[cfg(feature = "chrono")]
    pub timestamp: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone)]
pub struct Quantile {
    pub quantile: f64,
    pub value: f64,
}

/// A single `name="value"` label.
#[derive(Debug, Clone)]
pub struct LabelPair {
    pub name: String,
    pub value: String,
}

/// One measurement within a family: its identifying `label` set, its typed
/// `value`, and an optional sample `timestamp`.
#[derive(Debug, Clone)]
pub struct Metric {
    pub label: Vec<LabelPair>,
    pub value: MetricValue,
    #[cfg(not(feature = "chrono"))]
    pub timestamp: Option<Timestamp>,
    #[cfg(feature = "chrono")]
    pub timestamp: Option<chrono::DateTime<chrono::Utc>>,
}

/// A group of metrics sharing a `name`, `type`, and metadata — the unit a
/// `# TYPE`/`# HELP` block (text) or one protobuf message describes, and the
/// thing the payload parsers yield one at a time.
#[derive(Debug, Clone)]
pub struct MetricFamily {
    pub name: String,
    pub help: Option<String>,
    pub r#type: MetricType,
    pub metric: Vec<Metric>,
    pub unit: Option<String>,
}
