Native builds of Nabla, the ship2d viewer. The web version is at https://vini-fda.github.io/ship2d_optimization/

| Platform | File |
|---|---|
| macOS, Apple silicon | `ship2d-viewer-{{TAG}}-aarch64-apple-darwin.zip` |
| Linux, x86-64 | `ship2d-viewer-{{TAG}}-x86_64-unknown-linux-gnu.tar.gz` |
| Windows, 64-bit | `ship2d-viewer-{{TAG}}-x86_64-pc-windows-msvc.zip` |

- **macOS:** unzip and move `Nabla.app` to Applications. The app is signed with a Developer ID and notarized by Apple. <!-- notarized -->
- **macOS:** unzip and move `Nabla.app` to Applications. The app is not notarized, so macOS blocks the first launch. Allow it in System Settings → Privacy & Security → Open Anyway, or run `xattr -dr com.apple.quarantine "/Applications/Nabla.app"`. <!-- unsigned -->
- **Linux:** extract and run `./ship2d_graphics`. It needs X11 or Wayland.
- **Windows:** extract and run `ship2d_graphics.exe`. If SmartScreen warns, choose More info → Run anyway.

The stored solutions run out of the box. To open and run your own `.rs` solution, install Rust from https://rustup.rs: the viewer compiles it with `cargo`, and the first compile downloads this version of `ship2d_optimization`. `SHA256SUMS.txt` lists the checksums of the files.
