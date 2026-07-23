#![cfg(feature = "serde")]

use prometheus_scraper::{TextFormat, borrowed, owned, text_parse_family};

fn assert_owned_serde<T>()
where
    T: serde::Serialize + for<'de> serde::Deserialize<'de>,
{
}

fn assert_borrowed_serialize<T: serde::Serialize>() {}

#[test]
fn every_owned_type_supports_serde() {
    #[cfg(not(feature = "chrono"))]
    assert_owned_serde::<owned::Timestamp>();

    assert_owned_serde::<owned::Number>();
    assert_owned_serde::<owned::UnsignedNumber>();
    assert_owned_serde::<owned::Counter>();
    assert_owned_serde::<owned::Summary>();
    assert_owned_serde::<owned::Histogram>();
    assert_owned_serde::<owned::BucketSpan>();
    assert_owned_serde::<owned::NativeHistogram>();
    assert_owned_serde::<owned::NativeCounts>();
    assert_owned_serde::<owned::Info>();
    assert_owned_serde::<owned::State>();
    assert_owned_serde::<owned::StateSet>();
    assert_owned_serde::<owned::MetricValue>();
    assert_owned_serde::<owned::MetricType>();
    assert_owned_serde::<owned::BucketCount>();
    assert_owned_serde::<owned::BucketFloat>();
    assert_owned_serde::<owned::BucketInt>();
    assert_owned_serde::<owned::Exemplar>();
    assert_owned_serde::<owned::Quantile>();
    assert_owned_serde::<owned::LabelPair>();
    assert_owned_serde::<owned::Metric>();
    assert_owned_serde::<owned::MetricFamily>();
}

#[test]
fn every_borrowed_type_supports_serialization() {
    assert_borrowed_serialize::<borrowed::Counter<'static>>();
    assert_borrowed_serialize::<borrowed::Histogram<'static>>();
    assert_borrowed_serialize::<borrowed::NativeHistogram<'static>>();
    assert_borrowed_serialize::<borrowed::Info<'static>>();
    assert_borrowed_serialize::<borrowed::State<'static>>();
    assert_borrowed_serialize::<borrowed::StateSet<'static>>();
    assert_borrowed_serialize::<borrowed::MetricValue<'static>>();
    assert_borrowed_serialize::<borrowed::BucketCount<'static>>();
    assert_borrowed_serialize::<borrowed::BucketFloat<'static>>();
    assert_borrowed_serialize::<borrowed::BucketInt<'static>>();
    assert_borrowed_serialize::<borrowed::Exemplar<'static>>();
    assert_borrowed_serialize::<borrowed::LabelPair<'static>>();
    assert_borrowed_serialize::<borrowed::Metric<'static>>();
    assert_borrowed_serialize::<borrowed::MetricFamily<'static>>();
}

#[test]
fn borrowed_and_owned_models_share_a_serialized_shape() {
    let family = text_parse_family(
        "# TYPE requests gauge\n\
         # HELP requests Current requests\n\
         requests{method=\"GET\"} 7 1604676851\n",
        TextFormat::OpenMetrics,
    )
    .unwrap();

    let borrowed_json = serde_json::to_value(&family).unwrap();
    assert_eq!(borrowed_json["name"], serde_json::json!("requests"));
    assert_eq!(borrowed_json["type"], serde_json::json!("Gauge"));
    assert_eq!(
        borrowed_json["metric"][0]["value"]["Gauge"]["Int"],
        serde_json::json!(7)
    );
    assert!(!borrowed_json["metric"][0]["timestamp"].is_null());

    let owned_json = serde_json::to_value(family.into_owned()).unwrap();
    assert_eq!(borrowed_json, owned_json);

    let decoded: owned::MetricFamily = serde_json::from_value(owned_json.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), owned_json);
}
