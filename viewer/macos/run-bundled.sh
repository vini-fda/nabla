#!/usr/bin/env bash
# Cargo runner for macOS (see .cargo/config.toml): `cargo run [--release]` wraps the viewer in
# an app bundle first, so the Dock shows the Liquid Glass icon (a bare executable cannot).
# Every other binary (tests, benches, other crates) runs unchanged.
set -euo pipefail

bin="$1"
shift
if [ "$(basename "$bin")" != ship2d_graphics ]; then
    exec "$bin" "$@"
fi

here="$(cd "$(dirname "$0")" && pwd)"
app="$(SHIP2D_BIN="$bin" "$here/bundle.sh" | tail -1)"
exec "$app/Contents/MacOS/ship2d_graphics" "$@"
