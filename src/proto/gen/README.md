# Generated protobuf bindings — DO NOT EDIT

These files are generated **by hand** from [`../metrics.proto`](../metrics.proto)
with [`buffa`](https://github.com/anthropics/buffa)'s `protoc-gen-buffa` plugin
and committed to the repository. There is intentionally **no `build.rs`**, so
downstream users of this crate never run codegen — they only compile the
committed `.rs` plus the `buffa` runtime crate.

Regenerate only when `metrics.proto` changes:

```sh
./scripts/gen-proto.sh
```

One-time prerequisites (dev machine only):

```sh
brew install protobuf                  # provides `protoc` + google well-known types
cargo install --locked protoc-gen-buffa
```

## Files

| File | Contents |
| --- | --- |
| `io.prometheus.client.mod.rs` | Module entry point (`include!`d by `src/lib.rs` as `crate::proto`). Pulls in the two files below and re-exports the view types. |
| `metrics.rs` | Owned message + enum types (implement `buffa::Message`; support encode and owned decode). |
| `metrics.__view.rs` | Zero-copy `…View<'a>` types that borrow `&'a str` / `&'a [u8]` from the input buffer (implement `buffa::MessageView`; decode via `decode_view`). |

## Usage

```rust
use buffa::MessageView;
use prometheus_scraper::proto::MetricFamilyView;

let view = MetricFamilyView::decode_view(bytes)?; // borrows from `bytes`
println!("{:?}", view.name); // Option<&str>, no allocation
```
