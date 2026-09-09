# Implementation status

Updated: 9 September 2026

## Delivered

- M0 workspace, toolchain pin, licensing/provenance policy, ADR-001, Android
  and iOS native state/capability callback sources.
- M1 serializable Rust DTOs with tested camelCase wire fields, error contract,
  owner-scoped operation manager, caller-visible operation IDs, live
  cancellation routing, deterministic mock backend, idempotent cleanup, and
  TypeScript DTO/facade.
- M4 protocol core: fixed 14-byte v1 header, golden vectors, 20-byte value-limit
  fragmentation, physically possible fragment-count validation, metadata and
  payload accounting under one adapter budget, bounded partial-message count,
  30 s reassembly expiry, bounded complete/recent queues, stop-and-wait sender,
  two retries, honest ambiguous timeout outcome, session deduplication, and
  disconnect cleanup.
- Role-specific Tauri commands and permission sets for scan, connect, server,
  advertising and OS permission prompts; default access remains status plus
  owner cleanup. Cross-role commands are rejected again inside Rust.
- TypeScript package build and API tests.
- Generic example source for BLE lab, invitation flow, and unencrypted
  coordinator-relayed chat semantics. Chat envelopes are runtime-validated;
  history, deduplication and outbox state are bounded; receipts use coordinator
  sequence numbers; coordinator departure is explicit.
- The npm package contains a complete standalone MIT license file.
- Documentation, support matrix, contribution/security policies, CI definition,
  and release checklist.

## Tests actually run

- npm test: passed, 9 tests across the facade and offline-chat example,
  Node 22.20.0.
- TypeScript strict build and declaration generation: passed.
- npm audit: 0 vulnerabilities in the current development dependency tree.

## Not verified in this environment

- Rust formatting, compilation, clippy, and Rust unit tests: Rust is not
  installed on the host; an isolated toolchain download did not complete.
- Android compilation: Android SDK/Gradle wrapper is not available.
- iOS compilation: full Xcode is not available. Swift manifest validation was
  attempted, but the installed compiler and Command Line Tools SDK versions do
  not match.
- Any Bluetooth radio or physical-device scenario.

## Incomplete milestones

- M2: native Android/iOS scan, connect, GATT client/server, advertising, and
  targeted notification implementation.
- M2 security boundary: service-UUID scope configuration and enforcement is
  still required before enabling grants for untrusted WebViews.
- M3: concrete macOS, Windows, and Linux backends.
- M4 integration: peer state machine is not connected to native GATT callbacks.
- M5: packaged Tauri example apps and physical multi-peer qualification.
- M6: fresh-consumer crate/package install, SBOM/license scan, publishable npm
  owner/name, and release artifacts.

The production backend intentionally reports unknown capabilities and returns
Unsupported for radio operations. It never substitutes the mock backend.

## Next step

Install Rust 1.89 and platform SDKs, run the recorded CI commands, then implement
Android central/peripheral operations behind the Backend contract before
claiming mobile support.
