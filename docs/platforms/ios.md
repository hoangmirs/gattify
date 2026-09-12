# iOS

Target baseline: iOS 15. The app needs `NSBluetoothAlwaysUsageDescription`.

The Swift backend in `crates/tauri-plugin-gattify/ios/Sources` implements every
command and event of `docs/native-bridge.md` with CoreBluetooth. One serial
dispatch queue owns all state and is the delegate queue of both managers.

- The central and peripheral managers start on the first command that needs
  them, never at load. `getState` and `checkPermissions` read
  `CBManager.authorization` and create no manager. The managers do not show
  the system power alert.
- A command that creates a manager waits for its first state. `startScan` has
  no default deadline, so a scan that shows the permission prompt waits for the
  answer. `createServer` has a 5 s default, so a host should request the
  permission first.
- Each connection runs one GATT procedure at a time. When a procedure reaches
  its deadline, the connection closes with `connectionClosed`. A cancelled
  procedure holds the queue until its late callback arrives.
- A server keeps every value in the plugin and creates each characteristic with
  a nil value, so iOS asks the plugin for every read.

## Platform limits

- iOS reports a read and a notification through one callback. A read of a
  characteristic with a subscription on the connection rejects with `busy`.
- The advertised local name fits in 8 bytes next to a 128-bit service UUID.
  `startAdvertising` cuts longer names at a UTF-8 boundary.
- `LinkLimits` come from `maximumWriteValueLength(for: .withoutResponse)` when
  `didConnect` arrives. iOS can finish the MTU exchange later, so the first
  connection can report 20 bytes.
- A peripheral cannot disconnect a central. A server learns that a central left
  when iOS reports that the central unsubscribed.
- Background operation is unsupported.

## Verification

The Swift code compiles through `cargo build -p tauri-plugin-gattify --target
aarch64-apple-ios`. The XCTest suite in `ios/Tests` covers the JSON wire
format, command decoding, UUIDs, name cutting, scan routing, server write
batches, and the engine paths that need no radio. The simulator has no
Bluetooth, so radio behavior is verified on physical iPhones.
