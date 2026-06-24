#!/usr/bin/env bash
# Regenerate the committed buffa protobuf bindings from src/proto/metrics.proto.
#
# Run this ONLY when src/proto/metrics.proto changes. The generated output in
# src/proto/gen/ is committed to the repo, so downstream users of this crate
# never run codegen — there is intentionally no build.rs.
#
# One-time prerequisites (dev machine only, not a crate dependency):
#   - protoc            e.g. `brew install protobuf`
#   - protoc-gen-buffa  `cargo install --locked protoc-gen-buffa`
set -euo pipefail

# Run from the repo root regardless of where the script is invoked.
cd "$(dirname "$0")/.."

PLUGIN="$(command -v protoc-gen-buffa || echo "$HOME/.cargo/bin/protoc-gen-buffa")"
if [ ! -x "$PLUGIN" ]; then
  echo "error: protoc-gen-buffa not found. Install with:" >&2
  echo "       cargo install --locked protoc-gen-buffa" >&2
  exit 1
fi

# -I src/proto resolves metrics.proto; the second include resolves the google
# well-known types (google/protobuf/timestamp.proto) that metrics.proto imports.
INCLUDES=(-I src/proto)
if command -v brew >/dev/null 2>&1; then
  INCLUDES+=(-I "$(brew --prefix)/include")
fi

protoc \
  --plugin=protoc-gen-buffa="$PLUGIN" \
  --buffa_out=src/proto/gen \
  "${INCLUDES[@]}" \
  metrics.proto

echo "Regenerated src/proto/gen/ from src/proto/metrics.proto"
