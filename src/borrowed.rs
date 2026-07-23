//! The borrowed, zero-copy data model — what every parser returns.
//!
//! Each type here mirrors its counterpart in [`crate::owned`], but string fields
//! are [`Cow<'a, str>`](std::borrow::Cow) that borrow straight from the input
//! buffer (`Cow::Owned` only where an escape sequence forced an allocation). That
//! makes parsing allocation-light, but ties every family to the `&'a [u8]` it was
//! parsed from.
//!
//! When a value must outlive that buffer — to store it, send it across threads,
//! or return it — call [`into_owned`](MetricFamily::into_owned) (or rely on
//! [`From`]) to convert to the matching [`owned`](crate::owned) type. The
//! [`Decoder`](crate::Decoder) and the async `Client` expose owned-result
//! methods so callers usually never touch the lifetime directly.
//!
//! With the `serde` feature, every type in this module implements
//! `serde::Serialize`. Deserialization is provided only by [`crate::owned`],
//! whose allocation-backed fields can accept data from every serde format
//! without weakening this module's borrowing contract.

use std::borrow::Cow;

#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[derive(Debug)]
pub struct Counter<'a> {
    pub value: super::owned::UnsignedNumber,
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

#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[derive(Debug)]
pub struct Histogram<'a> {
    pub sample_sum: Option<super::owned::Number>,
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

#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[derive(Debug)]
pub struct NativeHistogram<'a> {
    pub schema: i32,
    pub zero_threshold: f64,
    pub sample_sum: Option<super::owned::Number>,
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

#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[derive(Debug)]
pub struct Info<'a> {
    pub labels: Vec<LabelPair<'a>>,
}

impl Info<'_> {
    pub fn into_owned(self) -> super::owned::Info {
        super::owned::Info {
            labels: self.labels.into_iter().map(LabelPair::into_owned).collect(),
        }
    }
}

#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[derive(Debug)]
pub struct State<'a> {
    pub name: Cow<'a, str>,
    pub enabled: bool,
}

impl State<'_> {
    pub fn into_owned(self) -> super::owned::State {
        super::owned::State {
            name: self.name.into_owned(),
            enabled: self.enabled,
        }
    }
}

#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[derive(Debug)]
pub struct StateSet<'a> {
    pub states: Vec<State<'a>>,
}

impl StateSet<'_> {
    pub fn into_owned(self) -> super::owned::StateSet {
        super::owned::StateSet {
            states: self.states.into_iter().map(State::into_owned).collect(),
        }
    }
}

#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[derive(Debug)]
pub enum MetricValue<'a> {
    Counter(Counter<'a>),
    Gauge(super::owned::Number),
    Summary(super::owned::Summary),
    Untyped(super::owned::Number),
    Histogram(Histogram<'a>),
    GaugeHistogram(Histogram<'a>),
    NativeHistogram(NativeHistogram<'a>),
    HybridHistogram {
        classic: Histogram<'a>,
        native: NativeHistogram<'a>,
    },
    StateSet(StateSet<'a>),
    Info(Info<'a>),
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
            MetricValue::StateSet(v) => super::owned::MetricValue::StateSet(v.into_owned()),
            MetricValue::Info(v) => super::owned::MetricValue::Info(v.into_owned()),
        }
    }
}

#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[derive(Debug)]
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

#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[derive(Debug)]
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

#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[derive(Debug)]
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

#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[derive(Debug)]
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

#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[derive(Debug)]
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

#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[derive(Debug)]
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

/// A group of metrics sharing a name, type, and metadata, borrowing from the
/// parsed buffer. The root type every parser yields; see the
/// [module docs](self) for the borrowing contract and
/// [`into_owned`](Self::into_owned) to detach from the buffer.
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[derive(Debug)]
pub struct MetricFamily<'a> {
    pub name: Cow<'a, str>,
    pub help: Option<Cow<'a, str>>,
    pub r#type: super::owned::MetricType,
    pub metric: Vec<Metric<'a>>,
    pub unit: Option<Cow<'a, str>>,
}

impl MetricFamily<'_> {
    /// Convert into an [`owned::MetricFamily`](super::owned::MetricFamily),
    /// copying every borrowed string so the result no longer references the
    /// input buffer.
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
