#!/usr/bin/env bash
# Builds `target/release/Nabla.app`: the native viewer as a macOS app bundle, with the
# layered Icon Composer icon (`nabla_macOS.icon`) that macOS 26 renders with Liquid Glass.
# Needs Xcode (for `actool`). Open the result with `open "target/release/Nabla.app"`.
#
# With `SHIP2D_BIN=<path to a built ship2d_graphics>` it skips `cargo build` and bundles that
# binary next to itself instead (used by `run-bundled.sh`, the cargo runner on macOS).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
bin="${SHIP2D_BIN:-$root/target/release/ship2d_graphics}"
app="$(dirname "$bin")/Nabla.app"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$here/../Cargo.toml" | head -1)"

# Fast path for repeated runs: the bundle is current, only the binary changed.
if [ -n "${SHIP2D_BIN:-}" ] && [ -f "$app/Contents/Info.plist" ] \
    && [ -z "$(find "$here/nabla_macOS.icon" "$here/bundle.sh" "$here/../Cargo.toml" \
        -newer "$app/Contents/Info.plist" -print -quit)" ]; then
    cp "$bin" "$app/Contents/MacOS/ship2d_graphics"
    codesign --force --sign - "$app" >&2
    echo "$app"
    exit 0
fi

if [ -z "${SHIP2D_BIN:-}" ]; then
    cargo build --release -p ship2d_graphics --manifest-path "$root/Cargo.toml"
fi

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$bin" "$app/Contents/MacOS/ship2d_graphics"

# Compile the icon: Assets.car (layered, Liquid Glass) plus a flat .icns fallback. Xcode
# versions before 26 cannot compile `.icon` files; then build a flat .icns from the PNG.
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
if xcrun actool "$here/nabla_macOS.icon" \
    --compile "$app/Contents/Resources" \
    --app-icon nabla_macOS \
    --platform macosx --target-device mac --minimum-deployment-target 11.0 \
    --enable-on-demand-resources NO --development-region en \
    --output-partial-info-plist "$work/icon.plist" >"$work/actool.log" 2>&1 \
    && [ -f "$app/Contents/Resources/Assets.car" ] && [ "${SHIP2D_FLAT_ICON:-0}" != 1 ]; then
    icon_name="<key>CFBundleIconName</key>
    <string>nabla_macOS</string>"
else
    echo "warning: no Liquid Glass icon (nabla_macOS.icon needs Xcode 26's actool); using a flat icon" >&2
    rm -f "$app/Contents/Resources/Assets.car"
    iconset="$work/nabla_macOS.iconset"
    mkdir "$iconset"
    for size in 16 32 128 256 512; do
        sips -s format png -z "$size" "$size" "$here/../assets/app-icon-macos.png" \
            --out "$iconset/icon_${size}x${size}.png" >/dev/null
        sips -s format png -z "$((size * 2))" "$((size * 2))" "$here/../assets/app-icon-macos.png" \
            --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
    done
    iconutil -c icns "$iconset" -o "$app/Contents/Resources/nabla_macOS.icns"
    icon_name=""
fi

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>Nabla</string>
    <key>CFBundleDisplayName</key>
    <string>Nabla</string>
    <key>CFBundleIdentifier</key>
    <string>io.github.vini-fda.ship2d-viewer</string>
    <key>CFBundleExecutable</key>
    <string>ship2d_graphics</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>$version</string>
    <key>CFBundleVersion</key>
    <string>$version</string>
    <key>CFBundleIconFile</key>
    <string>nabla_macOS</string>
    $icon_name
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
PLIST

# Ad-hoc signature, so the bundle runs on Apple Silicon.
codesign --force --sign - "$app" >&2
echo "$app"
