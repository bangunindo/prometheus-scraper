pub mod owned;
pub mod borrowed;

/// Generated protobuf types for the Prometheus `io.prometheus.client` schema.
///
/// This module is generated **by hand** from `src/proto/metrics.proto` using the
/// `protoc-gen-buffa` plugin and committed to the repo — there is no `build.rs`,
/// so downstream users never run codegen. See `src/proto/gen/README.md` for the
/// exact regeneration command (only needed on the rare occasions the `.proto`
/// changes).
///
/// The `…View<'a>` types are zero-copy: they borrow `&'a str` / `&'a [u8]`
/// straight from the input buffer. Decode via [`buffa::MessageView::decode_view`],
/// e.g. `proto::MetricFamilyView::decode_view(bytes)`.
#[allow(
    non_camel_case_types,
    non_upper_case_globals,
    dead_code,
    unused_imports,
    unused_qualifications,
    clippy::derivable_impls,
    clippy::match_single_binding,
    clippy::uninlined_format_args,
    clippy::doc_lazy_continuation,
    clippy::module_inception
)]
#[rustfmt::skip]
mod proto {
    include!("proto/gen/io.prometheus.client.mod.rs");
}

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
}
