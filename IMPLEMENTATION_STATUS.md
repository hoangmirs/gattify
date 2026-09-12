# Implementation status

Updated: 12 September 2026

## Delivered

- M0 workspace, toolchain pin, provenance policy, ADR-001 and ADR-002.
- M1 serializable Rust DTOs with tested camelCase wire fields, error contract,
  owner-scoped operation manager, caller-visible operation IDs, live
  cancellation routing, idempotent cleanup, and the TypeScript DTO/facade.
- M4 protocol core: fixed 14-byte v1 header, golden vectors, fragmentation,
  bounded reassembly, stop-and-wait sender with two retries, honest ambiguous
  timeout outcome, session deduplication. The ACK deadline counts from the last
  written fragment, so a slow link does not retransmit forever.
- Sub-project 2, the event path and the peer driver:
  - Backends raise events through an `EventSink`. The native layer sends event
    envelopes through the channel Rust gives it with `setEventChannel`.
  - Events of a `gattify-peer:` owner go to the peer driver. Every other event
    reaches only its owner webview, through the Tauri channels the webview
    registers with `listen_events`.
  - The peer driver hosts and dials peers: Info check, HELLO and HELLO_ACK,
    DATA and ACK with retransmission, CLOSE, lost links, a second HELLO from
    one central, the 10 s dial timeout. One writer task per peer keeps frames
    in order.
  - `gattify:scope` lists the service UUIDs of an app. `ScopeGuard` enforces
    each row of the scope table and remembers the characteristic handles each
    connection may use.
  - A page load or a closed window releases everything its webview held. New
    commands of that webview wait until the release ends.
  - `MockAir` links two mock backends and enforces characteristic properties
    and value limits.
- Sub-project 3, the Android backend: Kotlin implements every command and event
  of `docs/native-bridge.md`, with a GATT procedure queue per connection, the
  MTU request, notification flow control, prepared writes and runtime
  permissions.
- Sub-project 4, the iOS backend: Swift implements every command and event of
  `docs/native-bridge.md` on CoreBluetooth, with lazy managers, a GATT
  procedure queue per connection, notification flow control and whole-batch
  server writes.
- `docs/native-bridge.md`: the contract between Rust and both native layers.
- The lab app is the two-phone test harness: adapter state and permissions,
  host, scan and join, chat with the time from send to ACK, and a log.
- The TypeScript peer API: `listen`, `onClose`, one handle per peer, early
  messages kept for the first `onMessage`, and events replayed when they race
  endpoint creation or a dial reply.
- ChatGPT (Codex) reviewed the contract, the Rust driver and wiring, the iOS
  backend and the Android backend. Every finding was checked; the valid ones
  are fixed.

## Tests actually run

- cargo test --workspace --all-features: 84 tests passed.
- cargo test --workspace --no-default-features: 80 tests passed.
- cargo clippy --workspace --all-targets --all-features -- -D warnings: clean,
  also for the aarch64-apple-ios target.
- cargo test --manifest-path examples/gattify-lab/src-tauri/Cargo.toml: 9 IPC
  tests passed against Tauri's mock runtime, including the scope and the peer
  commands.
- npm test: 17 facade and peer tests, 5 offline-chat tests.
- XCTest on an iOS 26 simulator: 75 tests passed.
- The lab APK built for aarch64 Android, and `./gradlew
  :tauri-plugin-gattify:testDebugUnitTest`: 75 Kotlin tests passed.
  `lintDebug`: no issues.
- The lab app built for iOS devices, signed for development, installed and
  launched on an iPhone 16 Pro running iOS 26.6.1. Adapter state, the
  Bluetooth prompt, all three permissions and scanning worked.
- Hardware: the iPhone 16 Pro hosted and the macOS probe joined. The handshake
  and complete messages in both directions passed, including 4 KiB from the
  host. The run found ACK starvation and 20-byte frames; both are fixed and
  the ACK fix has a MockAir test. See
  `docs/platforms/test-results/2026-09-12-iphone-host-mac-joiner.md`.

## Not verified yet

- The two fixes above on hardware.
- The iPhone as a joiner, and two iPhones with each other.
- Any Android device.

## Incomplete milestones

- M3: concrete macOS, Windows and Linux backends (out of scope for v0.1).
- M5: physical multi-peer qualification on both platforms.
- M6: fresh-consumer crate/package install and SBOM/license scan.
- Sub-project 5, release preparation.

Desktop builds keep an explicit Unsupported backend. It never substitutes the
mock backend.

## Next step

Run the lab app on two phones: each phone as host and as joiner, a short
message and a 4 KiB message each way, a close from each side. Record the
results in `docs/platforms/test-results/`.
