Native builds of Nabla, the viewer for the 2D ship's control policies. The web version is at https://vini-fda.github.io/nabla/

| Platform | File |
|---|---|
| macOS, Apple silicon | `nabla-{{TAG}}-aarch64-apple-darwin.zip` |
| Linux, x86-64 | `nabla-{{TAG}}-x86_64-unknown-linux-gnu.tar.gz` |
| Windows, 64-bit | `nabla-{{TAG}}-x86_64-pc-windows-msvc.zip` |

- **macOS:** unzip and move `Nabla.app` to Applications. The app is signed with a Developer ID and notarized by Apple. <!-- notarized -->
- **macOS:** unzip and move `Nabla.app` to Applications. The app is not notarized, so macOS blocks the first launch. Allow it in System Settings → Privacy & Security → Open Anyway, or run `xattr -dr com.apple.quarantine "/Applications/Nabla.app"`. <!-- unsigned -->
- **Linux:** extract and run `./nabla`. It needs X11 or Wayland.
- **Windows:** extract and run `nabla.exe`. If SmartScreen warns, choose More info → Run anyway.

The multiplayer server, to fly with others:

| Platform | File |
|---|---|
| macOS, Apple silicon | `nabla-server-{{TAG}}-aarch64-apple-darwin.tar.gz` |
| Linux, x86-64 | `nabla-server-{{TAG}}-x86_64-unknown-linux-gnu.tar.gz` |

Extract and run `./nabla-server`, which logs the address to connect to. On macOS it is not notarized, so if you downloaded it with a browser, first run `xattr -d com.apple.quarantine nabla-server`. See [Running a server](https://github.com/vini-fda/nabla#running-a-server) for its flags.

`SHA256SUMS.txt` lists the checksums of the files.
