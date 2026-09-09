# Implementation status

Updated: 8 September 2026

## Delivered

- M0 workspace, toolchain pin, licensing/provenance policy, ADR-001, Android
  and iOS native state/capability callback sources.
- M1 serializable Rust DTOs, error contract, owner-scoped operation manager,
  deterministic mock backend, idempotent cleanup, and TypeScript DTO/facade.
- M4 protocol core: fixed 14-byte v1 header, golden vectors, 20-byte value-limit
  fragmentation, bounded reassembly/queues, stop-and-wait sender, two retries,
  honest ambiguous timeout outcome, session deduplication, and disconnect
  cleanup.
- TypeScript package build and API tests.
- Generic example source for BLE lab, invitation flow, and unencrypted
  coordinator-relayed chat semantics.
- Documentation, support matrix, contribution/security policies, CI definition,
  and release checklist.

## Tests actually run

- npm test: passed, 4 tests across the facade and offline-chat example,
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
- M2 security boundary: split the non-status Tauri command envelope into
  role-specific commands/scopes before enabling scan/connect/server grants;
  current role permission sets are scaffolding and all map to allow-execute.
- M3: concrete macOS, Windows, and Linux backends.
- M4 integration: peer state machine is not connected to native GATT callbacks.
- M5: packaged Tauri example apps and physical multi-peer qualification.
- M6: fresh-consumer crate/package install, SBOM/license scan, publishable owner
  names, and release artifacts.

The production backend intentionally reports unknown capabilities and returns
Unsupported for radio operations. It never substitutes the mock backend.

## Next step

Install Rust 1.89 and platform SDKs, run the recorded CI commands, then implement
Android central/peripheral operations behind the Backend contract before
claiming mobile support.
