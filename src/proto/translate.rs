//! Translation from the generated zero-copy protobuf `…View` types into the
//! crate's [`borrowed`](crate::borrowed) representation.
//!
//! Everything here is crate-private: the only entry point is the
//! `TryFrom<MetricFamilyView>` impl for [`borrowed::MetricFamily`], reached via
//! `borrowed::MetricFamily::try_from(view)`.

use std::borrow::Cow;

use crate::Error;
use crate::borrowed::{
    BucketCount, BucketFloat, BucketInt, Counter, Exemplar, Histogram, LabelPair, Metric,
    MetricFamily, MetricValue, NativeHistogram,
};
use crate::owned;

use super::{
    BucketSpanView, ExemplarView, HistogramView, LabelPairView, MetricFamilyView, MetricType,
    MetricView,
};

fn proto_infer_type(view: &MetricFamilyView) -> Result<owned::MetricType, Error> {
    match view.r#type {
        Some(MetricType::Counter) => Ok(owned::MetricType::Counter),
        Some(MetricType::Gauge) => Ok(owned::MetricType::Gauge),
        Some(MetricType::Summary) => Ok(owned::MetricType::Summary),
        Some(MetricType::Untyped) => Ok(owned::MetricType::Untyped),
        Some(MetricType::Histogram) => {
            if !view.metric.is_empty()
                && let histogram = &view.metric.first().unwrap().histogram
                && histogram.is_set()
            {
                match (!histogram.bucket.is_empty(), histogram.schema.is_some()) {
                    (true, true) => Ok(owned::MetricType::HybridHistogram),
                    (true, false) => Ok(owned::MetricType::Histogram),
                    (false, true) => Ok(owned::MetricType::NativeHistogram),
                    (false, false) => Ok(owned::MetricType::Histogram),
                }
            } else {
                Ok(owned::MetricType::Histogram)
            }
        }
        Some(MetricType::GaugeHistogram) => Ok(owned::MetricType::GaugeHistogram),
        None => Err(Error::MissingField("MetricFamily: type".into())),
    }
}

fn normalize_ts(seconds: &mut i64, nanos: &mut i32) {
    if *nanos < 0 || *nanos >= 1_000_000_000 {
        let extra_seconds = nanos.div_euclid(1_000_000_000);
        let extra_nanos = nanos.rem_euclid(1_000_000_000);
        *seconds = seconds.saturating_add(extra_seconds as i64);
        *nanos = extra_nanos;
    }
}

#[cfg(not(feature = "chrono"))]
fn proto_translate_ts(is_set: bool, mut seconds: i64, mut nanos: i32) -> Option<owned::Timestamp> {
    if !is_set {
        return None;
    }
    normalize_ts(&mut seconds, &mut nanos);
    Some(owned::Timestamp { seconds, nanos })
}

#[cfg(feature = "chrono")]
fn proto_translate_ts(
    is_set: bool,
    mut seconds: i64,
    mut nanos: i32,
) -> Option<chrono::DateTime<chrono::Utc>> {
    use chrono::TimeZone;
    if !is_set {
        return None;
    }
    normalize_ts(&mut seconds, &mut nanos);
    chrono::Utc.timestamp_opt(seconds, nanos as u32).single()
}

#[cfg(not(feature = "chrono"))]
fn proto_translate_ts_ms(ts: Option<i64>) -> Option<owned::Timestamp> {
    ts.map(|timestamp| owned::Timestamp {
        seconds: timestamp / 1000,
        nanos: (timestamp % 1000) as i32 * 1_000_000,
    })
}

#[cfg(feature = "chrono")]
fn proto_translate_ts_ms(ts: Option<i64>) -> Option<chrono::DateTime<chrono::Utc>> {
    use chrono::TimeZone;
    ts.and_then(|ts| chrono::Utc.timestamp_millis_opt(ts).single())
}

fn proto_translate_label<'a>(label: &LabelPairView<'a>) -> Result<LabelPair<'a>, Error> {
    Ok(LabelPair {
        name: Cow::Borrowed(
            label
                .name
                .ok_or_else(|| Error::MissingField("LabelPair: name".into()))?,
        ),
        value: Cow::Borrowed(label.value.unwrap_or("")),
    })
}

fn proto_translate_exemplar<'a>(
    is_set: bool,
    exemplar: &ExemplarView<'a>,
) -> Result<Option<Exemplar<'a>>, Error> {
    if !is_set {
        return Ok(None);
    }
    proto_translate_exemplar_inner(exemplar).map(Some)
}

fn proto_translate_exemplar_inner<'a>(exemplar: &ExemplarView<'a>) -> Result<Exemplar<'a>, Error> {
    Ok(Exemplar {
        value: exemplar
            .value
            .ok_or_else(|| Error::MissingField("Exemplar: value".into()))?,
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
    })
}

fn proto_translate_bucket_span(span: &BucketSpanView<'_>) -> Result<owned::BucketSpan, Error> {
    // A no-op span is `(offset 0, length 0)`, and proto2 may elide either
    // scalar when it equals its default, so an absent field is a genuine 0
    // rather than missing data.
    Ok(owned::BucketSpan {
        offset: span.offset.unwrap_or(0),
        length: span.length.unwrap_or(0),
    })
}

/// Translate the conventional (classic) part of a histogram: the explicit
/// `bucket` list plus its `sample_count`/`sample_sum`. Used on its own for
/// `Histogram`/`GaugeHistogram` and as the classic half of a hybrid.
fn proto_translate_classic_histogram<'a>(
    histogram: &HistogramView<'a>,
) -> Result<Histogram<'a>, Error> {
    // Float histograms carry their counts in the `*_float` fields; integer
    // histograms use the plain ones. The presence of a float count anywhere
    // is what marks the whole histogram as float.
    let is_float = histogram.sample_count_float.is_some_and(|v| v > 0.0)
        || histogram.zero_count_float.is_some_and(|v| v > 0.0)
        || !histogram.positive_count.is_empty()
        || !histogram.negative_count.is_empty();
    let counts = if is_float {
        BucketCount::Float {
            sample_count: histogram.sample_count_float,
            buckets: histogram
                .bucket
                .iter()
                .map(|bucket| {
                    Ok(BucketFloat {
                        cumulative_count: bucket
                            .cumulative_count_float
                            .or_else(|| bucket.cumulative_count.map(|count| count as f64))
                            .ok_or_else(|| {
                                Error::MissingField("Bucket: cumulative_count".into())
                            })?,
                        upper_bound: bucket
                            .upper_bound
                            .ok_or_else(|| Error::MissingField("Bucket: upper_bound".into()))?,
                        exemplar: proto_translate_exemplar(
                            bucket.exemplar.is_set(),
                            &bucket.exemplar,
                        )?,
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
        }
    } else {
        BucketCount::Int {
            sample_count: histogram.sample_count,
            buckets: histogram
                .bucket
                .iter()
                .map(|bucket| {
                    Ok(BucketInt {
                        cumulative_count: bucket.cumulative_count.ok_or_else(|| {
                            Error::MissingField("Bucket: cumulative_count".into())
                        })?,
                        upper_bound: bucket
                            .upper_bound
                            .ok_or_else(|| Error::MissingField("Bucket: upper_bound".into()))?,
                        exemplar: proto_translate_exemplar(
                            bucket.exemplar.is_set(),
                            &bucket.exemplar,
                        )?,
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
        }
    };
    Ok(Histogram {
        sample_sum: histogram.sample_sum,
        counts,
        created_timestamp: proto_translate_ts(
            histogram.created_timestamp.is_set(),
            histogram.created_timestamp.seconds,
            histogram.created_timestamp.nanos,
        ),
    })
}

/// Translate the native (sparse) part of a histogram: the `schema`, zero
/// bucket, and positive/negative spans with their per-bucket counts. Used on
/// its own for `NativeHistogram` and as the native half of a hybrid.
fn proto_translate_native_histogram<'a>(
    histogram: &HistogramView<'a>,
) -> Result<NativeHistogram<'a>, Error> {
    // Integer native histograms encode counts as deltas (`*_delta`), float
    // ones as absolute counts (`*_count`); the float-typed fields likewise
    // mark the histogram as float.
    let is_float = histogram.sample_count_float.is_some()
        || histogram.zero_count_float.is_some()
        || !histogram.positive_count.is_empty()
        || !histogram.negative_count.is_empty();
    let counts = if is_float {
        owned::NativeCounts::Float {
            sample_count: histogram.sample_count_float,
            zero_count: histogram.zero_count_float.unwrap_or(0.0),
            positive_spans: histogram
                .positive_span
                .iter()
                .map(proto_translate_bucket_span)
                .collect::<Result<Vec<_>, _>>()?,
            positive_counts: histogram.positive_count.to_vec(),
            negative_spans: histogram
                .negative_span
                .iter()
                .map(proto_translate_bucket_span)
                .collect::<Result<Vec<_>, _>>()?,
            negative_counts: histogram.negative_count.to_vec(),
        }
    } else {
        owned::NativeCounts::Int {
            sample_count: histogram.sample_count,
            zero_count: histogram.zero_count.unwrap_or(0),
            positive_spans: histogram
                .positive_span
                .iter()
                .map(proto_translate_bucket_span)
                .collect::<Result<Vec<_>, _>>()?,
            positive_deltas: histogram.positive_delta.to_vec(),
            negative_spans: histogram
                .negative_span
                .iter()
                .map(proto_translate_bucket_span)
                .collect::<Result<Vec<_>, _>>()?,
            negative_deltas: histogram.negative_delta.to_vec(),
        }
    };
    Ok(NativeHistogram {
        schema: histogram
            .schema
            .ok_or_else(|| Error::MissingField("Histogram: schema".into()))?,
        zero_threshold: histogram.zero_threshold.unwrap_or(0.0),
        sample_sum: histogram.sample_sum,
        counts,
        exemplars: histogram
            .exemplars
            .iter()
            .map(proto_translate_exemplar_inner)
            .collect::<Result<Vec<_>, _>>()?,
        created_timestamp: proto_translate_ts(
            histogram.created_timestamp.is_set(),
            histogram.created_timestamp.seconds,
            histogram.created_timestamp.nanos,
        ),
    })
}

fn proto_translate_metric<'a>(
    view: &MetricView<'a>,
    family_type: &owned::MetricType,
) -> Result<Metric<'a>, Error> {
    let label = view
        .label
        .iter()
        .map(proto_translate_label)
        .collect::<Result<Vec<_>, _>>()?;
    let value = if view.gauge.is_set() {
        MetricValue::Gauge(
            view.gauge
                .value
                .ok_or_else(|| Error::MissingField("Gauge: value".into()))?,
        )
    } else if view.counter.is_set() {
        let value = view
            .counter
            .value
            .ok_or_else(|| Error::MissingField("Counter: value".into()))?;
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
        MetricValue::Summary(owned::Summary {
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
                    Ok(owned::Quantile {
                        quantile: q
                            .quantile
                            .ok_or_else(|| Error::MissingField("Summary: quantile".into()))?,
                        value: q
                            .value
                            .ok_or_else(|| Error::MissingField("Summary: value".into()))?,
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
        })
    } else if view.histogram.is_set() {
        let histogram = &view.histogram;
        // A `GAUGE_HISTOGRAM` family is indistinguishable from a classic
        // histogram by its fields alone — only the family type marks it — so
        // it's handled separately. Otherwise the shape of the histogram
        // decides: explicit `bucket`s mean a classic part, a set `schema`
        // means a native part, and both together make a hybrid.
        if matches!(family_type, owned::MetricType::GaugeHistogram) {
            MetricValue::GaugeHistogram(proto_translate_classic_histogram(histogram)?)
        } else {
            match (!histogram.bucket.is_empty(), histogram.schema.is_some()) {
                (true, true) => MetricValue::HybridHistogram {
                    classic: proto_translate_classic_histogram(histogram)?,
                    native: proto_translate_native_histogram(histogram)?,
                },
                (false, true) => {
                    MetricValue::NativeHistogram(proto_translate_native_histogram(histogram)?)
                }
                (true, false) | (false, false) => {
                    MetricValue::Histogram(proto_translate_classic_histogram(histogram)?)
                }
            }
        }
    } else if view.untyped.is_set() {
        MetricValue::Untyped(
            view.untyped
                .value
                .ok_or_else(|| Error::MissingField("Untyped: value".into()))?,
        )
    } else {
        return Err(Error::MissingField(
            "Metric: gauge, counter, summary, histogram, or untyped".into(),
        ));
    };
    Ok(Metric {
        label,
        value,
        timestamp: proto_translate_ts_ms(view.timestamp_ms),
    })
}

impl<'a> TryFrom<MetricFamilyView<'a>> for MetricFamily<'a> {
    type Error = Error;

    fn try_from(value: MetricFamilyView<'a>) -> Result<Self, Self::Error> {
        let r#type = proto_infer_type(&value)?;
        let metric = value
            .metric
            .iter()
            .map(|metric| proto_translate_metric(metric, &r#type))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            name: Cow::Borrowed(
                value
                    .name
                    .ok_or_else(|| Error::MissingField("MetricFamily: name".into()))?,
            ),
            help: value.help.map(Cow::Borrowed),
            r#type,
            metric,
            unit: value.unit.map(Cow::Borrowed),
        })
    }
}
