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
    pub count: f64,
    pub upper_bound: f64,
    pub exemplar: Option<Exemplar<'a>>,
}

impl BucketFloat<'_> {
    pub fn into_owned(self) -> super::owned::BucketFloat {
        super::owned::BucketFloat {
            count: self.count,
            upper_bound: self.upper_bound,
            exemplar: self.exemplar.map(Exemplar::into_owned),
        }
    }
}

pub struct BucketInt<'a> {
    pub count: u64,
    pub upper_bound: f64,
    pub exemplar: Option<Exemplar<'a>>,
}

impl BucketInt<'_> {
    pub fn into_owned(self) -> super::owned::BucketInt {
        super::owned::BucketInt {
            count: self.count,
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

fn proto_infer_type(
    view: &super::proto::MetricFamilyView,
) -> Result<super::owned::MetricType, super::Error> {
    match view.r#type {
        Some(super::proto::MetricType::Counter) => Ok(super::owned::MetricType::Counter),
        Some(super::proto::MetricType::Gauge) => Ok(super::owned::MetricType::Gauge),
        Some(super::proto::MetricType::Summary) => Ok(super::owned::MetricType::Summary),
        Some(super::proto::MetricType::Untyped) => Ok(super::owned::MetricType::Untyped),
        Some(super::proto::MetricType::Histogram) => {
            if !view.metric.is_empty()
                && let histogram = &view.metric.first().unwrap().histogram
                && histogram.is_set()
            {
                match (!histogram.bucket.is_empty(), histogram.schema.is_some()) {
                    (true, true) => Ok(super::owned::MetricType::HybridHistogram),
                    (true, false) => Ok(super::owned::MetricType::Histogram),
                    (false, true) => Ok(super::owned::MetricType::NativeHistogram),
                    (false, false) => Ok(super::owned::MetricType::Histogram),
                }
            } else {
                Ok(super::owned::MetricType::Histogram)
            }
        }
        Some(super::proto::MetricType::GaugeHistogram) => {
            Ok(super::owned::MetricType::GaugeHistogram)
        }
        None => Err(super::Error::MissingField("MetricFamily: type".into())),
    }
}

#[cfg(not(feature = "chrono"))]
fn proto_translate_ts(is_set: bool, seconds: i64, nanos: i32) -> Option<super::owned::Timestamp> {
    if !is_set {
        return None;
    }
    Some(super::owned::Timestamp { seconds, nanos })
}

#[cfg(feature = "chrono")]
fn proto_translate_ts(
    is_set: bool,
    seconds: i64,
    nanos: i32,
) -> Option<chrono::DateTime<chrono::Utc>> {
    use chrono::TimeZone;
    if !is_set {
        return None;
    }
    let nanos = if nanos < 0 { 0 } else { nanos as u32 };
    chrono::Utc.timestamp_opt(seconds, nanos).single()
}

#[cfg(not(feature = "chrono"))]
fn proto_translate_ts_ms(ts: Option<i64>) -> Option<super::owned::Timestamp> {
    ts.map(|timestamp| super::owned::Timestamp {
        seconds: timestamp / 1000,
        nanos: (timestamp % 1000) as i32 * 1_000_000,
    })
}

#[cfg(feature = "chrono")]
fn proto_translate_ts_ms(ts: Option<i64>) -> Option<chrono::DateTime<chrono::Utc>> {
    use chrono::TimeZone;
    ts.and_then(|ts| chrono::Utc.timestamp_millis_opt(ts).single())
}

fn proto_translate_label<'a>(
    label: &super::proto::LabelPairView<'a>,
) -> Result<LabelPair<'a>, super::Error> {
    Ok(LabelPair {
        name: Cow::Borrowed(
            label
                .name
                .ok_or_else(|| super::Error::MissingField("LabelPair: name".into()))?,
        ),
        value: Cow::Borrowed(label.value.unwrap_or("")),
    })
}

fn proto_translate_exemplar<'a>(
    is_set: bool,
    exemplar: &super::proto::ExemplarView<'a>,
) -> Result<Option<Exemplar<'a>>, super::Error> {
    if !is_set {
        return Ok(None);
    }
    Ok(Some(Exemplar {
        value: exemplar
            .value
            .ok_or_else(|| super::Error::MissingField("Exemplar: value".into()))?,
        timestamp: proto_translate_ts(
            exemplar.timestamp.is_set(),
            exemplar.timestamp.seconds,
            exemplar.timestamp.nanos,
        ),
        label: exemplar
            .label
            .iter()
            .map(proto_translate_label)
            .collect::<Result<Vec<_>, _>>()?,
    }))
}

fn proto_translate_metric<'a>(
    view: &super::proto::MetricView<'a>,
) -> Result<Metric<'a>, super::Error> {
    let label = view
        .label
        .iter()
        .map(proto_translate_label)
        .collect::<Result<Vec<_>, _>>()?;
    let value = if view.gauge.is_set() {
        MetricValue::Gauge(
            view.gauge
                .value
                .ok_or_else(|| super::Error::MissingField("Gauge: value".into()))?,
        )
    } else if view.counter.is_set() {
        let value = view
            .counter
            .value
            .ok_or_else(|| super::Error::MissingField("Counter: value".into()))?;
        MetricValue::Counter(Counter {
            created_timestamp: proto_translate_ts(
                view.counter.created_timestamp.is_set(),
                view.counter.created_timestamp.seconds,
                view.counter.created_timestamp.nanos,
            ),
            value,
            exemplar: proto_translate_exemplar(
                view.counter.exemplar.is_set(),
                &view.counter.exemplar,
            )?,
        })
    } else if view.summary.is_set() {
        MetricValue::Summary(super::owned::Summary {
            sample_count: view.summary.sample_count,
            sample_sum: view.summary.sample_sum,
            created_timestamp: proto_translate_ts(
                view.summary.created_timestamp.is_set(),
                view.summary.created_timestamp.seconds,
                view.summary.created_timestamp.nanos,
            ),
            quantile: view
                .summary
                .quantile
                .iter()
                .map(|q| {
                    Ok(super::owned::Quantile {
                        quantile: q.quantile.ok_or_else(|| {
                            super::Error::MissingField("Summary: quantile".into())
                        })?,
                        value: q
                            .value
                            .ok_or_else(|| super::Error::MissingField("Summary: value".into()))?,
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
        })
    } else if view.histogram.is_set() {
        todo!()
    } else if view.untyped.is_set() {
        MetricValue::Untyped(
            view.untyped
                .value
                .ok_or_else(|| super::Error::MissingField("Untyped: value".into()))?,
        )
    } else {
        return Err(super::Error::MissingField(
            "Metric: gauge, counter, summary, histogram, or untyped".into(),
        ));
    };
    Ok(Metric {
        label,
        value,
        timestamp: proto_translate_ts_ms(view.timestamp_ms),
    })
}

impl From<MetricFamily<'_>> for super::owned::MetricFamily {
    fn from(family: MetricFamily<'_>) -> Self {
        family.into_owned()
    }
}

impl<'a> TryFrom<super::proto::MetricFamilyView<'a>> for MetricFamily<'a> {
    type Error = super::Error;

    fn try_from(value: super::proto::MetricFamilyView<'a>) -> Result<Self, Self::Error> {
        let metric = value
            .metric
            .iter()
            .map(|metric| proto_translate_metric(metric))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            name: Cow::Borrowed(
                value
                    .name
                    .ok_or_else(|| super::Error::MissingField("MetricFamily: name".into()))?,
            ),
            help: value.help.map(Cow::Borrowed),
            r#type: proto_infer_type(&value)?,
            metric,
            unit: value.unit.map(Cow::Borrowed),
        })
    }
}
