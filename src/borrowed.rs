use std::borrow::Cow;

pub struct Counter<'a> {
    pub value: f64,
    pub exemplar: Option<Exemplar<'a>>,
    #[cfg(not(feature = "chrono"))]
    pub created_timestamp: Option<super::owned::Timestamp>,
    #[cfg(feature = "chrono")]
    pub created_timestamp: Option<chrono::DateTime<chrono::Utc>>,
}

impl Counter<'_> {
    pub fn into_owned(self) -> super::owned::Counter {
        super::owned::Counter {
            value: self.value,
            exemplar: self.exemplar.map(|e| e.into_owned()),
            created_timestamp: self.created_timestamp,
        }
    }
}

pub struct Histogram<'a> {
    pub sample_sum: Option<f64>,
    pub counts: BucketCount<'a>,
    #[cfg(not(feature = "chrono"))]
    pub created_timestamp: Option<super::owned::Timestamp>,
    #[cfg(feature = "chrono")]
    pub created_timestamp: Option<chrono::DateTime<chrono::Utc>>,
}

impl Histogram<'_> {
    pub fn into_owned(self) -> super::owned::Histogram {
        super::owned::Histogram {
            sample_sum: self.sample_sum,
            counts: self.counts.into_owned(),
            created_timestamp: self.created_timestamp,
        }
    }
}

pub struct NativeHistogram<'a> {
    pub schema: i32,
    pub zero_threshold: f64,
    pub sample_sum: Option<f64>,
    pub counts: super::owned::NativeCounts,
    pub exemplars: Vec<Exemplar<'a>>,
    #[cfg(not(feature = "chrono"))]
    pub created_timestamp: Option<super::owned::Timestamp>,
    #[cfg(feature = "chrono")]
    pub created_timestamp: Option<chrono::DateTime<chrono::Utc>>,
}

impl NativeHistogram<'_> {
    pub fn into_owned(self) -> super::owned::NativeHistogram {
        super::owned::NativeHistogram {
            schema: self.schema,
            zero_threshold: self.zero_threshold,
            sample_sum: self.sample_sum,
            counts: self.counts,
            exemplars: self
                .exemplars
                .into_iter()
                .map(Exemplar::into_owned)
                .collect(),
            created_timestamp: self.created_timestamp,
        }
    }
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

impl MetricValue<'_> {
    pub fn into_owned(self) -> super::owned::MetricValue {
        match self {
            MetricValue::Counter(v) => super::owned::MetricValue::Counter(v.into_owned()),
            MetricValue::Gauge(v) => super::owned::MetricValue::Gauge(v),
            MetricValue::Summary(v) => super::owned::MetricValue::Summary(v),
            MetricValue::Untyped(v) => super::owned::MetricValue::Untyped(v),
            MetricValue::Histogram(v) => super::owned::MetricValue::Histogram(v.into_owned()),
            MetricValue::GaugeHistogram(v) => {
                super::owned::MetricValue::GaugeHistogram(v.into_owned())
            }
            MetricValue::NativeHistogram(v) => {
                super::owned::MetricValue::NativeHistogram(v.into_owned())
            }
            MetricValue::HybridHistogram { classic, native } => {
                super::owned::MetricValue::HybridHistogram {
                    classic: classic.into_owned(),
                    native: native.into_owned(),
                }
            }
        }
    }
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

impl BucketCount<'_> {
    pub fn into_owned(self) -> super::owned::BucketCount {
        match self {
            BucketCount::Int {
                sample_count,
                buckets,
            } => super::owned::BucketCount::Int {
                sample_count,
                buckets: buckets.into_iter().map(BucketInt::into_owned).collect(),
            },
            BucketCount::Float {
                sample_count,
                buckets,
            } => super::owned::BucketCount::Float {
                sample_count,
                buckets: buckets.into_iter().map(BucketFloat::into_owned).collect(),
            },
        }
    }
}

pub struct BucketFloat<'a> {
    pub cumulative_count: f64,
    pub upper_bound: f64,
    pub exemplar: Option<Exemplar<'a>>,
}

impl BucketFloat<'_> {
    pub fn into_owned(self) -> super::owned::BucketFloat {
        super::owned::BucketFloat {
            cumulative_count: self.cumulative_count,
            upper_bound: self.upper_bound,
            exemplar: self.exemplar.map(Exemplar::into_owned),
        }
    }
}

pub struct BucketInt<'a> {
    pub cumulative_count: u64,
    pub upper_bound: f64,
    pub exemplar: Option<Exemplar<'a>>,
}

impl BucketInt<'_> {
    pub fn into_owned(self) -> super::owned::BucketInt {
        super::owned::BucketInt {
            cumulative_count: self.cumulative_count,
            upper_bound: self.upper_bound,
            exemplar: self.exemplar.map(Exemplar::into_owned),
        }
    }
}

pub struct Exemplar<'a> {
    pub label: Vec<LabelPair<'a>>,
    pub value: f64,
    #[cfg(not(feature = "chrono"))]
    pub timestamp: Option<super::owned::Timestamp>,
    #[cfg(feature = "chrono")]
    pub timestamp: Option<chrono::DateTime<chrono::Utc>>,
}

impl Exemplar<'_> {
    pub fn into_owned(self) -> super::owned::Exemplar {
        super::owned::Exemplar {
            label: self.label.into_iter().map(LabelPair::into_owned).collect(),
            value: self.value,
            timestamp: self.timestamp,
        }
    }
}

pub struct LabelPair<'a> {
    pub name: Cow<'a, str>,
    pub value: Cow<'a, str>,
}

impl LabelPair<'_> {
    pub fn into_owned(self) -> super::owned::LabelPair {
        super::owned::LabelPair {
            name: self.name.into_owned(),
            value: self.value.into_owned(),
        }
    }
}

pub struct Metric<'a> {
    pub label: Vec<LabelPair<'a>>,
    pub value: MetricValue<'a>,
    #[cfg(not(feature = "chrono"))]
    pub timestamp: Option<super::owned::Timestamp>,
    #[cfg(feature = "chrono")]
    pub timestamp: Option<chrono::DateTime<chrono::Utc>>,
}

impl Metric<'_> {
    pub fn into_owned(self) -> super::owned::Metric {
        super::owned::Metric {
            label: self.label.into_iter().map(LabelPair::into_owned).collect(),
            value: self.value.into_owned(),
            timestamp: self.timestamp,
        }
    }
}

pub struct MetricFamily<'a> {
    pub name: Cow<'a, str>,
    pub help: Option<Cow<'a, str>>,
    pub r#type: super::owned::MetricType,
    pub metric: Vec<Metric<'a>>,
    pub unit: Option<Cow<'a, str>>,
}

impl MetricFamily<'_> {
    pub fn into_owned(self) -> super::owned::MetricFamily {
        super::owned::MetricFamily {
            name: self.name.into_owned(),
            help: self.help.map(|s| s.into_owned()),
            r#type: self.r#type,
            metric: self.metric.into_iter().map(Metric::into_owned).collect(),
            unit: self.unit.map(|s| s.into_owned()),
        }
    }
}

impl From<MetricFamily<'_>> for super::owned::MetricFamily {
    fn from(family: MetricFamily<'_>) -> Self {
        family.into_owned()
    }
}
