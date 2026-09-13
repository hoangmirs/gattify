# ADR-003: Desktop radio backends for macOS and Windows

Status: accepted
Date: 2026-09-13

## Context

ADR-002 kept a desktop backend that returns Unsupported, and asked for a new
record before a desktop radio backend. A Tauri app for macOS or Windows builds
with the plugin, but every radio command fails.

The peer protocol needs both roles on each device. A host runs a GATT server,
advertises, and notifies each central on its own. A joiner scans, connects,
discovers, subscribes and writes. `docs/native-bridge.md` also fixes the IDs,
the owner rules, one GATT procedure per connection, notification flow control
and the assembly of server writes. Kotlin on Android and Swift on iOS implement
it, and the iOS backend has passed part of its hardware run.

Tauri has a Swift and Kotlin plugin API on mobile only. A desktop plugin is
Rust. Tauri generates its Swift API package only for Android and iOS builds,
and the iOS package of this plugin depends on it.

## Decision

macOS runs the Swift engine of the iOS backend. CoreBluetooth is the same API
on both systems, and the engine uses nothing that is iOS-only. `build.rs`
compiles `ios/Sources`, without the Tauri entry point `GattifyPlugin.swift`,
and `macos/Bridge.swift` with `swiftc` into a static library. The bridge
exports two C functions that carry the JSON of `docs/native-bridge.md`.
`src/macos.rs` calls them. It is the only Rust code that may be unsafe, so the
workspace lint `unsafe_code` moves from `forbid` to `deny`.

Windows gets a backend in Rust on the WinRT Bluetooth APIs, through the
`windows` crate at the version Tauri already uses. It implements the `Backend`
trait directly, without a JSON bridge, and follows the same contract. It has no
unsafe code.

Linux keeps the Unsupported backend. A BlueZ backend needs its own record.

## Options considered

| Option | Roles | Code to maintain | Extra build toolchain | Main risk |
| --- | --- | --- | --- | --- |
| macOS: the iOS Swift engine through a C ABI (chosen) | All | A small Swift bridge and a Rust FFI module | Xcode or the Command Line Tools, which a macOS build needs anyway | Unsafe calls in one Rust module |
| macOS: Rust on CoreBluetooth through `objc2` | All | A third CoreBluetooth implementation | None | Drift from iOS; unsafe delegate classes in every file |
| macOS: a SwiftPM package through `swift-rs` | All | As chosen | As chosen | The iOS package needs Tauri's iOS API, which macOS builds lack, and a second package cannot hold sources outside its root |
| macOS: the engine as a helper process | All | A Swift executable and a pipe protocol | As chosen | Every app bundles and signs a sidecar |
| Windows: Rust on WinRT through `windows` (chosen) | All that the adapter supports | A Rust backend | None | No hardware run yet |
| Windows: a C# or C++/WinRT helper | All that the adapter supports | A backend in another language | .NET or C++/WinRT | Every app ships a helper or a runtime |
| Both: `btleplug` or `bluest` | Central only | Glue | None | No GATT server and no advertising, so no host |
| Both: a central crate with a peripheral crate | All, split | Glue across two libraries | None | Two threading and ID models, and neither has the procedure queue or the flow-controlled notify of the contract |

The chosen macOS option reuses reviewed and tested code: the 75 XCTests of the
engine pass on macOS unchanged. The chosen Windows option adds a third
implementation of the contract. That cost buys the host role, which no
existing crate offers, without a second language or runtime in every app.

## Consequences

- macOS and iOS share one CoreBluetooth implementation. A fix lands on both,
  and the XCTest suite runs on both.
- An app on macOS needs `NSBluetoothAlwaysUsageDescription` in
  `src-tauri/Info.plist`, and a sandboxed app needs the Bluetooth entitlement.
- A macOS build compiles Swift. Cross-building a macOS app from another system
  stays impossible, as it already is for Tauri.
- Unsafe code outside `src/macos.rs` still fails the build. Each exception
  needs its own `allow` next to a `SAFETY` comment.
- The Windows backend's platform-neutral parts are unit-tested on every host.
  Its WinRT calls compile in CI, but only Windows hardware runs them.
- CI gains a macOS job and a Windows job.
- Neither desktop backend counts as verified in `docs/support-matrix.md` until
  it passes a hardware run with the lab app.

This record replaces the desktop backend decision of ADR-002.
