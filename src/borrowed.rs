use std::borrow::Cow;

pub struct Counter<'a> {
    pub value: f64,
    pub exemplar: Option<Exemplar<'a>>,
    #[cfg(not(feature = "chrono"))]
    pub created_timestamp_ns: Option<i128>,
    #[cfg(feature = "chrono")]
    pub created_timestamp: Option<chrono::DateTime<chrono::Utc>>,
}

pub struct Histogram<'a> {
    pub sample_sum: Option<f64>,
    pub counts: BucketCount<'a>,
    #[cfg(not(feature = "chrono"))]
    pub created_timestamp_ns: Option<i128>,
    #[cfg(feature = "chrono")]
    pub created_timestamp: Option<chrono::DateTime<chrono::Utc>>,
}

pub struct NativeHistogram<'a> {
    pub schema: i32,
    pub zero_threshold: f64,
    pub sample_sum: Option<f64>,
    pub counts: super::owned::NativeCounts,
    pub exemplars: Vec<Exemplar<'a>>,
    #[cfg(not(feature = "chrono"))]
    pub created_timestamp_ns: Option<i128>,
    #[cfg(feature = "chrono")]
    pub created_timestamp: Option<chrono::DateTime<chrono::Utc>>,
}

pub enum MetricValue<'a> {
    Counter(Counter<'a>),
    Gauge(f64),
    Summary(super::owned::Summary),
    Untyped(f64),
    Histogram(Histogram<'a>),
    GaugeHistogram(Histogram<'a>),
    NativeHistogram(NativeHistogram<'a>),
    HybridHistogram {
        classic: Histogram<'a>,
        native: NativeHistogram<'a>,
    },
}

pub enum BucketCount<'a> {
    Int {
        sample_count: Option<u64>,
        buckets: Vec<BucketInt<'a>>,
    },
    Float {
        sample_count: Option<f64>,
        buckets: Vec<BucketFloat<'a>>,
    },
}

pub struct BucketFloat<'a> {
    pub count: f64,
    pub upper_bound: f64,
    pub exemplar: Option<Exemplar<'a>>,
}

pub struct BucketInt<'a> {
    pub count: u64,
    pub upper_bound: f64,
    pub exemplar: Option<Exemplar<'a>>,
}

pub struct Exemplar<'a> {
    pub label: Vec<LabelPair<'a>>,
    pub value: f64,
    #[cfg(not(feature = "chrono"))]
    pub timestamp_ns: Option<i128>,
    #[cfg(feature = "chrono")]
    pub timestamp: Option<chrono::DateTime<chrono::Utc>>,
}

pub struct LabelPair<'a> {
    pub name: Cow<'a, str>,
    pub value: Cow<'a, str>,
}

pub struct Metric<'a> {
    pub label: Vec<LabelPair<'a>>,
    pub value: MetricValue<'a>,
    #[cfg(not(feature = "chrono"))]
    pub timestamp_ms: Option<i64>,
    #[cfg(feature = "chrono")]
    pub timestamp: Option<chrono::DateTime<chrono::Utc>>,
}

pub struct MetricFamily<'a> {
    pub name: Cow<'a, str>,
    pub help: Option<Cow<'a, str>>,
    pub r#type: super::owned::MetricType,
    pub metric: Vec<Metric<'a>>,
    pub unit: Option<Cow<'a, str>>,
}
