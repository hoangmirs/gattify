# Changelog

## Unreleased

## 0.1.0-alpha.1 - 2026-09-12

This prerelease puts the radio to work on Android and iOS. An iPhone hosted a
peer and exchanged complete messages in both directions with a Mac. Android
has not run on a device yet.

- Added the Android backend in Kotlin and the iOS backend in Swift. Both
  implement every command and event of `docs/native-bridge.md`.
- Added the peer driver: `createEndpoint` with `listen`, `dial`, `send`,
  `onMessage`, `onClose` and `close` exchange complete messages between phones.
- Added the `gattify:scope` permission. An app lists its service UUIDs, and
  every radio command stays inside them. An empty scope rejects every radio
  command.
- Events reach only the webview that owns them, through a channel that the
  webview registers with `listen_events`. `gattify:default` allows it.
- A page reload or a closed window releases every scan, connection, server and
  peer of the old page.
- `gattify://scan-result` carries `{ device }`.
- The ACK deadline counts from the last written fragment, and ACK frames leave
  ahead of queued DATA frames.
- Added the lab app as a two-phone test harness, and a macOS probe that joins
  or hosts a lab peer.

Not verified on hardware: the iPhone as a joiner, two phones together, any
Android device, and the two fixes from the first iPhone run. This prerelease
skips the license review, the SBOM and the fresh-consumer install, by the
owner's decision.

## 0.1.0-alpha.0 - 2026-09-12

This prerelease claims the package names. Every radio command returns `Unsupported`.

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

