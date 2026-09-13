# Provenance log

This table records the external material consulted while writing the source in
this repository. No third-party code is copied here.

| Source | Accessed | Purpose | License / terms | Code copied | Notice |
| --- | --- | --- | --- | --- | --- |
| https://v2.tauri.app/develop/plugins/ | 2026-09-08 | Plugin lifecycle and command conventions | Tauri website terms; Tauri code Apache-2.0/MIT | No | Link only |
| https://v2.tauri.app/develop/plugins/develop-mobile/ | 2026-09-08 | Kotlin/Swift plugin boundary and callback guidance | Tauri website terms; Tauri code Apache-2.0/MIT | No | Link only |
| https://developer.android.com/reference/android/bluetooth/BluetoothGattServer | 2026-09-08 | Android GATT server API behavior | Android documentation terms | No | Link only |
| https://developer.android.com/develop/connectivity/bluetooth/bt-permissions | 2026-09-08 | Android permission model | Android documentation terms | No | Link only |
| https://developer.apple.com/documentation/corebluetooth | 2026-09-08 | CoreBluetooth state and manager APIs | Apple developer documentation terms | No | Link only |
| https://learn.microsoft.com/en-us/windows/apps/develop/devices-sensors/gatt-server | 2026-09-08 | Windows GATT server feasibility | Microsoft documentation terms | No | Link only |
| https://bluez.readthedocs.io/en/latest/gatt-api/ | 2026-09-08 | BlueZ exported GATT model | Documentation terms | No | Link only |
| tauri 2.11.5 crate source: `src/ipc/channel.rs`, `src/plugin/mobile.rs`, `mobile/*/Channel.*` | 2026-09-10 | Whether a channel from `Channel::new` receives native messages | Apache-2.0 OR MIT | No | Link only |
| https://github.com/tauri-apps/tauri at `tauri-cli-v2.11.4`: `crates/tauri-cli/templates/plugin`, `crates/tauri-plugin` | 2026-09-10 | Plugin build script, `links` key, Android and iOS project layout | Apache-2.0 OR MIT | No. The build files follow the template layout | Link only |
| https://github.com/tauri-apps/plugins-workspace at `v2`: `.github/workflows/test-android.yml`, `test-rust.yml`, `plugins/geolocation` | 2026-09-10 | CI method that compiles Kotlin and Swift, and the shape of a channel argument | Apache-2.0 OR MIT | No | Link only |
| swift-rs 1.0.8 crate source: `src-rs/build.rs`; tauri-utils 2.9.3 crate source: `src/build.rs` | 2026-09-13 | Which link search paths and libraries a Rust build needs for a Swift static library on macOS | Apache-2.0 OR MIT | No. `build.rs` asks `swiftc -print-target-info` itself | Link only |
| tauri-plugin 2.6.3 crate source: `src/build/mobile.rs`; tauri-codegen 2.6.3 crate source: `src/context.rs` | 2026-09-13 | When Tauri generates its Swift API package, and whether `tauri dev` embeds `Info.plist` on macOS | Apache-2.0 OR MIT | No | Link only |
| https://learn.microsoft.com/en-us/uwp/api/windows.devices.bluetooth.genericattributeprofile.gattserviceprovideradvertisingparameters and its `servicedata` page | 2026-09-13 | `IsConnectable`, `IsDiscoverable`, `ServiceData` and `StartedWithoutAllAdvertisementData` | Microsoft documentation terms | No | Link only |
| https://learn.microsoft.com/en-us/windows/apps/develop/devices-sensors/gatt-server | 2026-09-13 | Windows GATT server deferrals, responses, subscribed clients and restricted services | Microsoft documentation terms | No | Link only |
| https://learn.microsoft.com/en-us/uwp/api/windows.devices.bluetooth.bluetoothadapter | 2026-09-13 | The Windows version of each adapter property | Microsoft documentation terms | No | Link only |
| https://learn.microsoft.com/en-us/uwp/api/windows.devices.bluetooth.genericattributeprofile.gattreadrequest and `gattwriterequest` | 2026-09-13 | Read offsets, write options, and the missing prepared-write boundary | Microsoft documentation terms | No | Link only |
| Microsoft Q&A questions 840622, 953751 and 182735 on learn.microsoft.com/answers | 2026-09-13 | How much service data fits beside a service UUID, where Windows puts it, and whether a server learns the last part of a write | Microsoft Q&A terms | No | Link only |
| https://github.com/microsoft/Windows-universal-samples at `main`: `Samples/BluetoothLE/cs/Scenario3_ServerForeground.xaml.cs` | 2026-09-13 | What `IsDiscoverable` and `IsConnectable` mean, advertisement status handling, the deferral pattern | MIT | No. The sample is C# | Link only |
| windows 0.61.3, windows-core 0.61.2, windows-future 0.2.1, windows-collections 0.2.0, windows-result 0.3.4 and windows-strings 0.4.2 crate sources | 2026-09-13 | WinRT API signatures, awaitable async operations, COM apartment setup, GUID and string conversion | MIT OR Apache-2.0 | No | Link only |
| Prior knowledge of the WinRT backends of https://github.com/hbldh/bleak and https://github.com/kevincar/bless, not opened | 2026-09-13 | The order of the Windows connect and disconnect steps, and one service provider per service | MIT | No | Link only |

Dependency versions are recorded in Cargo.lock and package-lock.json after
resolution. THIRD_PARTY_NOTICES.md records direct dependency licenses. Release
requires a transitive SPDX or CycloneDX inventory and license review.
