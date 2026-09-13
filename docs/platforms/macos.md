# macOS

The macOS backend is the iOS backend. `build.rs` compiles the Swift engine in
`crates/tauri-plugin-gattify/ios/Sources` for macOS, without the Tauri iOS entry
point `GattifyPlugin.swift`, and links it into the app as a static library.
`crates/tauri-plugin-gattify/macos/Bridge.swift` replaces that entry point with
a C ABI that Rust calls from `src/macos.rs`. CoreBluetooth is the same API on
both systems, so every command, event and rule of `docs/native-bridge.md` and
`docs/platforms/ios.md` holds on macOS too.

## App requirements

- `NSBluetoothAlwaysUsageDescription` in the app's `Info.plist`. With Tauri,
  put it in `src-tauri/Info.plist`: Tauri merges that file into the bundle,
  and `tauri dev` embeds it into the development binary. macOS stops an app
  that starts a CoreBluetooth manager without it.
- A sandboxed app also needs the `com.apple.security.device.bluetooth`
  entitlement.
- The build machine needs Xcode or the Command Line Tools, as every macOS
  build of a Tauri app does. The Swift code compiles for the app's
  `MACOSX_DEPLOYMENT_TARGET`, or macOS 10.15 when it is unset.

## Platform limits

- The first command that starts a manager shows the macOS Bluetooth prompt.
  macOS remembers the answer for the app's code signature, so an unsigned
  development build can ask again after a rebuild.
- The advertised local name fits in 8 bytes next to a 128-bit service UUID,
  as on iOS.
- A Mac does not see its own advertisement. Test a Mac host against another
  device.
- Background operation is unsupported.

## Verification

- `cargo test --workspace --all-features` on a Mac compiles the engine, links
  it, and runs Rust tests that reach the Swift engine through the C ABI.
- `crates/tauri-plugin-gattify/macos/test.sh` runs the XCTest suite of
  `ios/Tests` on macOS against the same sources.
- The `macos` CI job runs both, clippy, and the lab app's IPC tests.
