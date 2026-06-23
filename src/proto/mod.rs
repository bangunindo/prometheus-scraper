//! Generated protobuf types for the Prometheus `io.prometheus.client` schema,
//! plus the translation from the zero-copy `…View` types into the crate's
//! [`borrowed`](crate::borrowed) representation.
//!
//! The generated code is produced **by hand** from `src/proto/metrics.proto`
//! using the `protoc-gen-buffa` plugin and committed to the repo — there is no
//! `build.rs`, so downstream users never run codegen. See
//! `src/proto/gen/README.md` for the exact regeneration command (only needed on
//! the rare occasions the `.proto` changes).
//!
//! The `…View<'a>` types are zero-copy: they borrow `&'a str` / `&'a [u8]`
//! straight from the input buffer. Decode via [`buffa::MessageView::decode_view`],
//! e.g. `proto::MetricFamilyView::decode_view(bytes)`.
//!
//! This module is crate-private: neither the generated types nor the translation
//! routines are part of the public API.

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
mod generated {
    include!("gen/io.prometheus.client.mod.rs");
}

pub(crate) use generated::*;

mod translate;
