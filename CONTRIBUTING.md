# Contributing to Gnobbi

Thank you for your interest in Gnobbi, a Matrix client for Linux, macOS, Windows, iOS and Android. Desktop and mobile share the egui UI in `crates/matrix-desktop`. iOS is available in TestFlight; Android is coming to Google Play.

## Before you start

- Open an issue describing the problem or proposal before starting a substantial pull request. This helps avoid work outside the project's scope.
- Use the official [Matrix Rust SDK](https://github.com/matrix-org/matrix-rust-sdk) for Matrix functionality. Custom implementations in its place are generally not accepted.
- Do not add telemetry or commit personal data, secrets or credentials.
- UI strings are written in English in the code and translated in the `locales/` catalogs. See [TRANSLATING.md](TRANSLATING.md).

## Build and test

From the repository root, on Linux:

```sh
cargo build --locked -p matrix-desktop --features linux-vault
cargo test --locked -p matrix-domain
cargo test --locked -p matrix-desktop
cargo clippy --locked --workspace --all-targets --features matrix-engine/sdk,matrix-desktop/linux-vault -- -D warnings
```

On macOS, use `mac-vault` instead of `linux-vault`; on Windows, use `windows-vault`. See the [README](README.md#building) for prerequisites. `Cargo.lock` is tracked: use `--locked` and update the lockfile only when dependencies actually change.

For mobile builds, the public packaging scripts provide the steps:

- iOS: on macOS with Rust and Xcode, `packaging/ios/bundle-device.sh` builds an unsigned device app. Signing is a separate step; see the script for prerequisites and options.
- Android: `packaging/android/build-apk.sh` documents how to build the Rust shared library first, then packages a debug APK using the Android SDK and JDK.

Pull requests should build, pass tests and introduce no Clippy warnings. For UI changes, include screenshots of both desktop and phone layouts.

## Reports

- For bugs and proposals, open an issue with reproduction steps, expected and observed behavior, and the version or commit.
- For vulnerabilities, see [SECURITY.md](SECURITY.md); do not post vulnerability details in a public issue.

## License

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in Gnobbi shall be dual-licensed under the [MIT license](LICENSE-MIT) or the [Apache License, Version 2.0](LICENSE-APACHE), at your option, without any additional terms or conditions.
