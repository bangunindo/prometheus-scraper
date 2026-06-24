#[cfg(not(feature = "chrono"))]
pub struct Timestamp {
    pub seconds: i64,
    pub nanos: i32,
}

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

pub struct Counter {
    pub value: UnsignedNumber,
    pub exemplar: Option<Exemplar>,
    #[cfg(not(feature = "chrono"))]
    pub created_timestamp: Option<Timestamp>,
    #[cfg(feature = "chrono")]
    pub created_timestamp: Option<chrono::DateTime<chrono::Utc>>,
}

pub struct Summary {
    pub sample_count: Option<u64>,
    pub sample_sum: Option<Number>,
    pub quantile: Vec<Quantile>,
    #[cfg(not(feature = "chrono"))]
    pub created_timestamp: Option<Timestamp>,
    #[cfg(feature = "chrono")]
    pub created_timestamp: Option<chrono::DateTime<chrono::Utc>>,
}

pub struct Histogram {
    pub sample_sum: Option<Number>,
    pub counts: BucketCount,
    #[cfg(not(feature = "chrono"))]
    pub created_timestamp: Option<Timestamp>,
    #[cfg(feature = "chrono")]
    pub created_timestamp: Option<chrono::DateTime<chrono::Utc>>,
}

pub struct BucketSpan {
    pub offset: i32,
    pub length: u32,
}

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

pub struct Info {
    /// The info label set (the payload). The sample value is always 1.
    pub labels: Vec<LabelPair>,
}

pub struct State {
    pub name: String,
    pub enabled: bool,
}

pub struct StateSet {
    pub states: Vec<State>,
}

pub enum MetricValue {
    Counter(Counter),
    Gauge(Number),
    Summary(Summary),
    Untyped(Number),
    Histogram(Histogram),
    GaugeHistogram(Histogram),
    NativeHistogram(NativeHistogram),
    HybridHistogram {
        classic: Histogram,
        native: NativeHistogram,
    },
    StateSet(StateSet),
    Info(Info),
}

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

pub struct BucketFloat {
    pub cumulative_count: f64,
    pub upper_bound: f64,
    pub exemplar: Option<Exemplar>,
}

pub struct BucketInt {
    pub cumulative_count: u64,
    pub upper_bound: f64,
    pub exemplar: Option<Exemplar>,
}

pub struct Exemplar {
    pub label: Vec<LabelPair>,
    pub value: f64,
    #[cfg(not(feature = "chrono"))]
    pub timestamp: Option<Timestamp>,
    #[cfg(feature = "chrono")]
    pub timestamp: Option<chrono::DateTime<chrono::Utc>>,
}

pub struct Quantile {
    pub quantile: f64,
    pub value: f64,
}

pub struct LabelPair {
    pub name: String,
    pub value: String,
}

pub struct Metric {
    pub label: Vec<LabelPair>,
    pub value: MetricValue,
    #[cfg(not(feature = "chrono"))]
    pub timestamp: Option<Timestamp>,
    #[cfg(feature = "chrono")]
    pub timestamp: Option<chrono::DateTime<chrono::Utc>>,
}

pub struct MetricFamily {
    pub name: String,
    pub help: Option<String>,
    pub r#type: MetricType,
    pub metric: Vec<Metric>,
    pub unit: Option<String>,
}
