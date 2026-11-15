# Releasing

Merging a version bump into `main` publishes a [GitHub Release](https://github.com/vini-fda/nabla/releases) with builds of the native app and the server. The app's builds are for macOS on Apple silicon (an app bundle), Linux x86-64 and Windows 64-bit, and the server's for the same macOS and Linux. Merges without a bump only run CI and deploy the web app.

To release, bump the workspace `version` in `Cargo.toml` (`[workspace.package]`, shared by every crate) and `Cargo.lock`, in the pull request or one of its own, and commit it:

```sh
cargo bump minor  # or major, or patch
```

After the merge, `.github/workflows/tag.yml` creates the tag `vX.Y.Z` if that version is newer than every existing tag, then starts `.github/workflows/release.yml` on it. The release first checks that the tag matches the workspace version and reruns CI. It then builds each target on a native runner and publishes the release with checksums and the notes from `.github/release-notes.md`.

These tags are made in Actions, so they aren't GPG-signed. If the workflow can't tag a version, `cargo tag` does the same by hand: on `main`, it also checks that the checkout has no uncommitted changes and is level with `origin/main`, and asks before pushing the tag.

If a release fails after its tag exists, rerun its failed jobs. If it never started, start it with `gh workflow run release.yml --ref vX.Y.Z`. A failure that needs a code fix takes a new bump, since the tag is on the broken commit.

## Signing the macOS app

With these secrets in the repository's `release` environment, the release signs `Nabla.app` with a Developer ID, notarizes it and staples the ticket, so Gatekeeper opens it without a warning. Without `MACOS_CERTIFICATE`, the app is signed ad-hoc and the release notes tell users how to allow it. The server binary is never signed with a Developer ID, and the notes say how to run it.

The signing runs in its own job, `sign-macos`, after the build: it checks out nothing and runs no cargo, so no build script or dependency ever runs next to the certificate. Only that job uses the environment, and the environment only deploys tags matching `v*.*.*` (Settings → Environments → release), so a workflow pushed on a branch cannot read the secrets.

| Secret | Value |
|---|---|
| `MACOS_CERTIFICATE` | A *Developer ID Application* certificate with its private key, exported from Keychain Access as `.p12`, in base64 |
| `MACOS_CERTIFICATE_PASSWORD` | The password of that `.p12` |
| `APPLE_API_KEY` | The contents of an App Store Connect API key, `AuthKey_<id>.p8` |
| `APPLE_API_KEY_ID` | That key's ID |
| `APPLE_API_ISSUER_ID` | The issuer ID shown above the list of keys |

1. Create the certificate in Xcode → Settings → Accounts → the team → Manage Certificates → + → Developer ID Application (only the team's Account Holder can), which puts it and its key in the login keychain. Export both from Keychain Access → My Certificates as `.p12`. An *Apple Distribution* certificate is for the App Store and cannot be notarized.
2. Create the API key in [App Store Connect](https://appstoreconnect.apple.com/access/integrations/api) → Users and Access → Integrations → Team Keys, with the Developer role. The `.p8` downloads only once.
3. Store them in the environment:

   ```sh
   base64 -i DeveloperID.p12 | gh secret set MACOS_CERTIFICATE --env release
   gh secret set MACOS_CERTIFICATE_PASSWORD --env release
   gh secret set APPLE_API_KEY --env release < AuthKey_XXXXXXXXXX.p8
   gh secret set APPLE_API_KEY_ID --env release
   gh secret set APPLE_API_ISSUER_ID --env release
   ```

To sign a local bundle the same way: `cargo xtask bundle --sign "Developer ID Application: <name> (<team>)"`.
