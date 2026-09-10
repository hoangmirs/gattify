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

Dependency versions are recorded in Cargo.lock and package-lock.json after
resolution. THIRD_PARTY_NOTICES.md records direct dependency licenses. Release
requires a transitive SPDX or CycloneDX inventory and license review.
