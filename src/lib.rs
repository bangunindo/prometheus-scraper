use std::fmt;

pub mod borrowed;
pub mod owned;
pub mod payload;
pub mod proto;
pub mod text;

mod frame;

pub use payload::{Decoder, Format, parse as parse_payload};
pub use proto::parse_family as proto_parse_family;
pub use text::parse_family as text_parse_family;
pub use text::TextFormat;

#[derive(Debug)]
pub enum Error {
    MissingField(String),
    InvalidFieldValue((String, String)),
    ProtoDecodeError(buffa::DecodeError),
    /// A text-exposition line that could not be parsed as a sample. Carries the
    /// offending line (without its trailing newline).
    InvalidLine(String),
    /// A text-exposition frame that was not valid UTF-8.
    InvalidUtf8(std::str::Utf8Error),
    /// A frame that could not be delimited: a protobuf length prefix that was
    /// malformed, or a frame truncated at end of input.
    IncompleteFrame,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::MissingField(field) => write!(f, "missing required field: {}", field),
            Error::InvalidFieldValue((field, value)) => {
                write!(f, "invalid field value: {} = {}", field, value)
            }
            Error::ProtoDecodeError(err) => write!(f, "protobuf decode error: {}", err),
            Error::InvalidLine(line) => write!(f, "invalid text exposition line: {}", line),
            Error::InvalidUtf8(err) => write!(f, "invalid UTF-8 in text frame: {}", err),
            Error::IncompleteFrame => write!(f, "incomplete or malformed frame"),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use crate::proto::{Gauge, LabelPair, Metric, MetricFamily, MetricFamilyView, MetricType};
    use buffa::{Message, MessageView};

    #[test]
    fn metric_family_decodes_as_zero_copy_view() {
        // Build an owned message and encode it to protobuf wire bytes.
        let owned = MetricFamily {
            name: Some("http_requests_total".into()),
            help: Some("Total HTTP requests".into()),
            r#type: Some(MetricType::COUNTER),
            metric: vec![Metric {
                label: vec![LabelPair {
                    name: Some("method".into()),
                    value: Some("GET".into()),
                    ..Default::default()
                }],
                gauge: Gauge {
                    value: Some(1.0),
                    ..Default::default()
                }
                .into(),
                ..Default::default()
            }],
            unit: Some("seconds".into()),
            ..Default::default()
        };
        let bytes = owned.encode_to_vec();

        // Decode a borrowed view straight from the buffer.
        let view = MetricFamilyView::decode_view(&bytes).unwrap();
        assert_eq!(view.name, Some("http_requests_total"));
        assert_eq!(view.unit, Some("seconds"));
        assert_eq!(view.r#type, Some(MetricType::COUNTER));
        assert_eq!(view.metric.len(), 1);

        let metric = view.metric.iter().next().unwrap();
        let label = metric.label.iter().next().unwrap();
        assert_eq!(label.name, Some("method"));
        assert_eq!(label.value, Some("GET"));

        // Prove the view is genuinely zero-copy: the decoded `&str` points *inside*
        // the input buffer rather than at a freshly allocated copy.
        let name_ptr = view.name.unwrap().as_ptr();
        let buf = bytes.as_ptr_range();
        assert!(
            buf.contains(&name_ptr),
            "string fields must borrow from the input buffer"
        );
    }

    use crate::owned::{BucketCount, MetricValue, NativeCounts, Number};
    use crate::proto::{Bucket, BucketSpan, Histogram};

    /// Round-trip a single-metric histogram family through the wire and the
    /// borrowed translation, returning the (necessarily owned) metric value.
    fn translate_histogram(family_type: MetricType, histogram: Histogram) -> MetricValue {
        let owned = MetricFamily {
            name: Some("test_histogram".into()),
            r#type: Some(family_type),
            metric: vec![Metric {
                histogram: histogram.into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let bytes = owned.encode_to_vec();
        let view = MetricFamilyView::decode_view(&bytes).unwrap();
        let family: crate::owned::MetricFamily = crate::borrowed::MetricFamily::try_from(view)
            .unwrap()
            .into();
        family.metric.into_iter().next().unwrap().value
    }

    #[test]
    fn classic_integer_histogram() {
        let value = translate_histogram(
            MetricType::HISTOGRAM,
            Histogram {
                sample_count: Some(3),
                sample_sum: Some(6.5),
                bucket: vec![
                    Bucket {
                        cumulative_count: Some(1),
                        upper_bound: Some(1.0),
                        ..Default::default()
                    },
                    Bucket {
                        cumulative_count: Some(3),
                        upper_bound: Some(f64::INFINITY),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
        );
        let MetricValue::Histogram(h) = value else {
            panic!("expected a classic histogram");
        };
        assert_eq!(h.sample_sum, Some(Number::Float(6.5)));
        let BucketCount::Int {
            sample_count,
            buckets,
        } = h.counts
        else {
            panic!("expected integer bucket counts");
        };
        assert_eq!(sample_count, Some(3));
        assert_eq!(buckets.len(), 2);
        assert_eq!(buckets[0].cumulative_count, 1);
        assert_eq!(buckets[0].upper_bound, 1.0);
        assert_eq!(buckets[1].cumulative_count, 3);
        assert!(buckets[1].upper_bound.is_infinite());
    }

    #[test]
    fn classic_float_histogram() {
        // The `*_float` count fields mark the histogram as float-valued.
        let value = translate_histogram(
            MetricType::HISTOGRAM,
            Histogram {
                sample_count_float: Some(3.0),
                sample_sum: Some(6.5),
                bucket: vec![Bucket {
                    cumulative_count_float: Some(2.5),
                    upper_bound: Some(1.0),
                    ..Default::default()
                }],
                ..Default::default()
            },
        );
        let MetricValue::Histogram(h) = value else {
            panic!("expected a classic histogram");
        };
        let BucketCount::Float {
            sample_count,
            buckets,
        } = h.counts
        else {
            panic!("expected float bucket counts");
        };
        assert_eq!(sample_count, Some(3.0));
        assert_eq!(buckets[0].cumulative_count, 2.5);
    }

    #[test]
    fn gauge_histogram_distinguished_by_family_type() {
        // Identical fields to a classic histogram — only the family type
        // tells them apart.
        let value = translate_histogram(
            MetricType::GAUGE_HISTOGRAM,
            Histogram {
                sample_count: Some(1),
                bucket: vec![Bucket {
                    cumulative_count: Some(1),
                    upper_bound: Some(f64::INFINITY),
                    ..Default::default()
                }],
                ..Default::default()
            },
        );
        assert!(matches!(value, MetricValue::GaugeHistogram(_)));
    }

    #[test]
    fn native_integer_histogram() {
        // No buckets + a schema => native; `*_delta` => integer counts.
        let value = translate_histogram(
            MetricType::HISTOGRAM,
            Histogram {
                schema: Some(2),
                zero_threshold: Some(0.001),
                zero_count: Some(4),
                sample_count: Some(10),
                sample_sum: Some(42.0),
                positive_span: vec![BucketSpan {
                    offset: Some(1),
                    length: Some(2),
                    ..Default::default()
                }],
                positive_delta: vec![3, -1],
                ..Default::default()
            },
        );
        let MetricValue::NativeHistogram(h) = value else {
            panic!("expected a native histogram");
        };
        assert_eq!(h.schema, 2);
        assert_eq!(h.zero_threshold, 0.001);
        let NativeCounts::Int {
            sample_count,
            zero_count,
            positive_spans,
            positive_deltas,
            ..
        } = h.counts
        else {
            panic!("expected integer native counts");
        };
        assert_eq!(sample_count, Some(10));
        assert_eq!(zero_count, 4);
        assert_eq!(positive_spans.len(), 1);
        assert_eq!(positive_spans[0].offset, 1);
        assert_eq!(positive_spans[0].length, 2);
        assert_eq!(positive_deltas, vec![3, -1]);
    }

    #[test]
    fn native_float_histogram() {
        // `*_count` / `*_float` fields => float native counts.
        let value = translate_histogram(
            MetricType::HISTOGRAM,
            Histogram {
                schema: Some(0),
                zero_count_float: Some(1.5),
                sample_count_float: Some(5.0),
                positive_span: vec![BucketSpan {
                    offset: Some(0),
                    length: Some(1),
                    ..Default::default()
                }],
                positive_count: vec![2.5],
                ..Default::default()
            },
        );
        let MetricValue::NativeHistogram(h) = value else {
            panic!("expected a native histogram");
        };
        let NativeCounts::Float {
            sample_count,
            zero_count,
            positive_counts,
            ..
        } = h.counts
        else {
            panic!("expected float native counts");
        };
        assert_eq!(sample_count, Some(5.0));
        assert_eq!(zero_count, 1.5);
        assert_eq!(positive_counts, vec![2.5]);
    }

    #[test]
    fn hybrid_histogram() {
        // Buckets *and* a schema => both halves are present.
        let value = translate_histogram(
            MetricType::HISTOGRAM,
            Histogram {
                sample_count: Some(7),
                sample_sum: Some(12.0),
                bucket: vec![Bucket {
                    cumulative_count: Some(7),
                    upper_bound: Some(f64::INFINITY),
                    ..Default::default()
                }],
                schema: Some(3),
                zero_count: Some(1),
                positive_span: vec![BucketSpan {
                    offset: Some(0),
                    length: Some(1),
                    ..Default::default()
                }],
                positive_delta: vec![2],
                ..Default::default()
            },
        );
        let MetricValue::HybridHistogram { classic, native } = value else {
            panic!("expected a hybrid histogram");
        };
        assert!(matches!(classic.counts, BucketCount::Int { .. }));
        assert_eq!(native.schema, 3);
        assert!(matches!(native.counts, NativeCounts::Int { .. }));
    }
}
