#!/usr/bin/env bash
# Run SANY and TLC on NablaNet.tla, the module that docs/multiplayer.typ embeds.
# Needs java and tla2tools.jar (TLA2TOOLS, default ~/tla/tla2tools.jar).
# Usage: docs/tla/check.sh [small|two-ships|large ...]   (default: all three)
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
jar=${TLA2TOOLS:-$HOME/tla/tla2tools.jar}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

cd "$here"
java -cp "$jar" tla2sany.SANY NablaNet.tla
for cfg in "${@:-small two-ships large}"; do
  for c in $cfg; do
    echo "== TLC: $c"
    java -XX:+UseParallelGC -cp "$jar" tlc2.TLC -workers auto -metadir "$work/states" \
      -config "$here/$c.cfg" NablaNet.tla
  done
done
