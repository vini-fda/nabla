#!/usr/bin/env bash
# Runs the same checks as CI (.github/workflows/ci.yml), plus the web build.
set -eux

cargo fmt --all -- --check
cargo clippy --quiet --workspace --all-targets -- -D warnings
cargo check --quiet -p ship2d_graphics --lib --target wasm32-unknown-unknown
cargo test --quiet --workspace --release
(cd viewer && trunk build --release)
