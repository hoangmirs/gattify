# Changelog

## Unreleased

- Added a release workflow. When the repository variable `RELEASE_ON_MERGE` is `true`, each merge into `develop` publishes a new minor version to crates.io and npm.
- Renamed the project to gattify and merged the Rust crates into `tauri-plugin-gattify`.
- Added workspace foundation, portable BLE contracts and deterministic mock.
- Added bounded peer framing, reassembly, ACK/retry and deduplication logic.
- Added TypeScript facade with raw and peer exports.
- Added Android and iOS native adapter-state reads.
- Added documentation, policy, examples and honest platform gates.
- Added the plugin build script and the Tauri native project layout.
- Added a native execute bridge on Android and iOS for status commands.
- Added the gattify lab app and CI jobs that compile the Kotlin and Swift code.
- Added tests for the Tauri command layer and the Android and iOS dispatch, and a CI fuzz run.

No native platform is claimed as production-ready in this release state.

