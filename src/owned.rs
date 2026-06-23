pub struct Counter {
    pub value: f64,
    pub exemplar: Option<Exemplar>,
    #[cfg(not(feature = "chrono"))]
    pub created_timestamp_ns: Option<i128>,
    #[cfg(feature = "chrono")]
    pub created_timestamp: Option<chrono::DateTime<chrono::Utc>>,
}

pub struct Summary {
    pub sample_count: Option<u64>,
    pub sample_sum: Option<f64>,
    pub quantile: Vec<Quantile>,
    #[cfg(not(feature = "chrono"))]
    pub created_timestamp_ns: Option<i128>,
    #[cfg(feature = "chrono")]
    pub created_timestamp: Option<chrono::DateTime<chrono::Utc>>,
}

pub struct Histogram {
    pub sample_sum: Option<f64>,
    pub counts: BucketCount,
    #[cfg(not(feature = "chrono"))]
    pub created_timestamp_ns: Option<i128>,
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
    pub sample_sum: Option<f64>,
    pub counts: NativeCounts,
    pub exemplars: Vec<Exemplar>,
    #[cfg(not(feature = "chrono"))]
    pub created_timestamp_ns: Option<i128>,
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

pub enum MetricValue {
    Counter(Counter),
    Gauge(f64),
    Summary(Summary),
    Untyped(f64),
    Histogram(Histogram),
    GaugeHistogram(Histogram),
    NativeHistogram(NativeHistogram),
    HybridHistogram {
        classic: Histogram,
        native: NativeHistogram,
    },
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
    pub count: f64,
    pub upper_bound: f64,
    pub exemplar: Option<Exemplar>,
}

pub struct BucketInt {
    pub count: u64,
    pub upper_bound: f64,
    pub exemplar: Option<Exemplar>,
}

pub struct Exemplar {
    pub label: Vec<LabelPair>,
    pub value: f64,
    #[cfg(not(feature = "chrono"))]
    pub timestamp_ns: Option<i128>,
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
    pub timestamp_ms: Option<i64>,
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
