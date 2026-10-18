//! The viewer as a macOS app bundle, `Nabla.app`, with the layered Icon Composer icon
//! (`viewer/macos/nabla_macOS.icon`) that macOS 26 renders with Liquid Glass. A bare
//! executable can only show a flat rounded icon in the Dock.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context as _, Result};
use xshell::{Shell, cmd};

use crate::project_root;

/// The binary's name, inside `Contents/MacOS`.
const EXECUTABLE: &str = "ship2d_graphics";

/// `Nabla.app`, next to the viewer binary `bin`.
fn app_path(bin: &Path) -> PathBuf {
    bin.with_file_name("Nabla.app")
}

/// Build the bundle from scratch next to `bin`, and return its path. `identity`: the keychain
/// identity to sign with; `None` signs ad-hoc.
pub fn create(sh: &Shell, bin: &Path, flat_icon: bool, identity: Option<&str>) -> Result<PathBuf> {
    let app = app_path(bin);
    if app.exists() {
        sh.remove_path(&app)?;
    }
    let resources = app.join("Contents/Resources");
    sh.create_dir(app.join("Contents/MacOS"))?;
    sh.create_dir(&resources)?;
    sh.copy_file(bin, app.join("Contents/MacOS").join(EXECUTABLE))?;

    let layered = !flat_icon && compile_layered_icon(sh, &resources)?;
    if !layered {
        compile_flat_icon(sh, &resources)?;
    }
    sh.write_file(
        app.join("Contents/Info.plist"),
        info_plist(&viewer_version()?, layered),
    )?;
    sign(sh, &app, identity)?;
    Ok(app)
}

/// Bring the bundle next to `bin` up to date for running `bin`, and return the binary inside
/// it. When nothing but the binary changed since the bundle was made, only the binary is
/// replaced: no icon compile.
pub fn refresh(sh: &Shell, bin: &Path) -> Result<PathBuf> {
    let app = app_path(bin);
    if is_current(&app)? {
        sh.copy_file(bin, app.join("Contents/MacOS").join(EXECUTABLE))?;
        sign(sh, &app, None)?;
    } else {
        create(sh, bin, false, None)?;
    }
    Ok(app.join("Contents/MacOS").join(EXECUTABLE))
}

/// Whether the bundle's Info.plist is newer than everything it was made from: the icon, the
/// viewer's version, and this module, which lays the bundle out.
fn is_current(app: &Path) -> Result<bool> {
    let Ok(made) = fs::metadata(app.join("Contents/Info.plist")).and_then(|m| m.modified()) else {
        return Ok(false);
    };
    let root = project_root();
    let mut inputs = vec![
        root.join("viewer/Cargo.toml"),
        root.join("xtask/src/bundle.rs"),
    ];
    collect_files(&root.join("viewer/macos/nabla_macOS.icon"), &mut inputs)?;
    for input in inputs {
        if modified(&input)? > made {
            return Ok(false);
        }
    }
    Ok(true)
}

fn modified(path: &Path) -> Result<SystemTime> {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .with_context(|| format!("reading the modification time of {}", path.display()))
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            collect_files(&path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

/// Compile the layered icon into `Assets.car` with Xcode's `actool`. Returns false, with a
/// warning, if that fails: only Xcode 26's `actool` can compile `.icon` files.
fn compile_layered_icon(sh: &Shell, resources: &Path) -> Result<bool> {
    let icon = project_root().join("viewer/macos/nabla_macOS.icon");
    let work = sh.create_temp_dir()?;
    let partial_plist = work.path().join("icon.plist");
    let compiled = cmd!(
        sh,
        "xcrun actool {icon} --compile {resources} --app-icon nabla_macOS
            --platform macosx --target-device mac --minimum-deployment-target 11.0
            --enable-on-demand-resources NO --development-region en
            --output-partial-info-plist {partial_plist}"
    )
    .quiet()
    .ignore_stdout()
    .ignore_stderr()
    .run()
    .is_ok();
    let assets = resources.join("Assets.car");
    if compiled && assets.exists() {
        return Ok(true);
    }
    warn("no Liquid Glass icon (nabla_macOS.icon needs Xcode 26's actool); using a flat icon");
    if assets.exists() {
        sh.remove_path(&assets)?;
    }
    Ok(false)
}

#[expect(
    clippy::print_stderr,
    reason = "a warning for the person running the task"
)]
fn warn(message: &str) {
    eprintln!("warning: {message}");
}

/// The flat fallback icon, `nabla_macOS.icns`, from the app icon PNG. macOS uses it where the
/// layered icon is missing (before macOS 26, or without Xcode 26).
fn compile_flat_icon(sh: &Shell, resources: &Path) -> Result<()> {
    let png = project_root().join("viewer/assets/app-icon-macos.png");
    let work = sh.create_temp_dir()?;
    let iconset = work.path().join("nabla_macOS.iconset");
    sh.create_dir(&iconset)?;
    for size in [16, 32, 128, 256, 512] {
        for (scale, suffix) in [(1, ""), (2, "@2x")] {
            let pixels = (size * scale).to_string();
            let out = iconset.join(format!("icon_{size}x{size}{suffix}.png"));
            cmd!(
                sh,
                "sips -s format png -z {pixels} {pixels} {png} --out {out}"
            )
            .quiet()
            .ignore_stdout()
            .run()?;
        }
    }
    let icns = resources.join("nabla_macOS.icns");
    cmd!(sh, "iconutil -c icns {iconset} -o {icns}").run()?;
    Ok(())
}

/// Sign the bundle with `identity`, or ad-hoc without one, which is enough to run on Apple
/// silicon but not for Gatekeeper. Notarization needs a Developer ID identity, the hardened
/// runtime and a secure timestamp. The viewer needs no entitlements: it only spawns `cargo`
/// and the binaries it builds, which the hardened runtime allows.
fn sign(sh: &Shell, app: &Path, identity: Option<&str>) -> Result<()> {
    match identity {
        None => cmd!(sh, "codesign --force --sign - {app}").quiet().run()?,
        Some(identity) => {
            cmd!(
                sh,
                "codesign --force --options runtime --timestamp --sign {identity} {app}"
            )
            .run()?;
            cmd!(sh, "codesign --verify --strict --verbose=2 {app}").run()?;
        }
    }
    Ok(())
}

/// The viewer's version, from `viewer/Cargo.toml`.
fn viewer_version() -> Result<String> {
    let manifest = project_root().join("viewer/Cargo.toml");
    let text = fs::read_to_string(&manifest)?;
    text.lines()
        .find_map(|line| {
            line.strip_prefix("version = \"")?
                .strip_suffix('"')
                .map(str::to_owned)
        })
        .with_context(|| format!("no `version = \"...\"` line in {}", manifest.display()))
}

/// `Contents/Info.plist`. `layered`: the bundle has the Liquid Glass icon (`Assets.car`), named
/// by `CFBundleIconName`; `CFBundleIconFile` is the flat fallback.
fn info_plist(version: &str, layered: bool) -> String {
    let icon_name = if layered {
        "\n    <key>CFBundleIconName</key>\n    <string>nabla_macOS</string>"
    } else {
        ""
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
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
    <string>{EXECUTABLE}</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>{version}</string>
    <key>CFBundleVersion</key>
    <string>{version}</string>
    <key>CFBundleIconFile</key>
    <string>nabla_macOS</string>{icon_name}
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
"#
    )
}
