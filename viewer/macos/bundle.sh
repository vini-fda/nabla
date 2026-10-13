#!/usr/bin/env bash
# Builds `target/release/ship2d viewer.app`: the native viewer as a macOS app bundle, with the
# layered Icon Composer icon (`nabla_macOS.icon`) that macOS 26 renders with Liquid Glass.
# Needs Xcode (for `actool`). Open the result with `open "target/release/ship2d viewer.app"`.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
app="$root/target/release/ship2d viewer.app"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$here/../Cargo.toml" | head -1)"

cargo build --release -p ship2d_graphics --manifest-path "$root/Cargo.toml"

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$root/target/release/ship2d_graphics" "$app/Contents/MacOS/ship2d_graphics"

# Compile the icon: Assets.car (layered, Liquid Glass) plus a flat .icns fallback.
partial="$(mktemp -d)"
trap 'rm -rf "$partial"' EXIT
xcrun actool "$here/nabla_macOS.icon" \
    --compile "$app/Contents/Resources" \
    --app-icon nabla_macOS \
    --platform macosx --target-device mac --minimum-deployment-target 11.0 \
    --enable-on-demand-resources NO --development-region en \
    --output-partial-info-plist "$partial/icon.plist" >/dev/null

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>ship2d viewer</string>
    <key>CFBundleDisplayName</key>
    <string>ship2d viewer</string>
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
    <key>CFBundleIconName</key>
    <string>nabla_macOS</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
PLIST

# Ad-hoc signature, so the bundle runs on Apple Silicon.
codesign --force --sign - "$app"
echo "$app"
