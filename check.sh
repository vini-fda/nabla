#!/usr/bin/env bash
# Runs the same checks as CI (.github/workflows/ci.yml), plus the web build.
set -eux

export RUSTFLAGS="-D warnings" RUSTDOCFLAGS="-D warnings"

cargo fmt --all -- --check
cargo doc --locked --quiet --workspace --no-deps --document-private-items
cargo clippy --locked --quiet --workspace --all-targets
cargo clippy --locked --quiet -p ship2d_graphics --lib --target wasm32-unknown-unknown
CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS=true \
    cargo test --locked --quiet --workspace --release
(cd viewer && trunk build --release)
