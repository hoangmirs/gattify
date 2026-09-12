# Android

Target baseline: API 26+, compile SDK 36, Java 17. The Kotlin backend in
`crates/tauri-plugin-gattify/android` implements every command and event of
`docs/native-bridge.md`. It compiles in CI, and its pure parts run as JUnit
tests. No radio behavior is verified on hardware yet.

## Structure

| File | Responsibility |
| --- | --- |
| `GattifyPlugin.kt` | Tauri commands `execute` and `setEventChannel`, permission aliases and the permission callback |
| `Backend.kt` | The handler thread, dispatch, readiness checks, owners, events, `cancel`, `closeOwner`, adapter broadcasts |
| `Central.kt` | Connections, the MTU request, discovery, reads, writes, subscriptions, one GATT procedure at a time per connection |
| `Scanner.kt` | One platform scan per `startScan`, throttled results, timeouts |
| `Peripheral.kt` | The shared `BluetoothGattServer`, stored values, CCCDs, long writes, notification flow control |
| `Advertiser.kt` | The one legacy advertisement, with the name as service data in the scan response |
| `Wire.kt`, `Payloads.kt`, `ExecuteResult.kt` | JSON replies, events and command payloads |
| `Procedures.kt`, `Queues.kt`, `Ids.kt`, `Gatt.kt`, `Uuids.kt`, `Names.kt`, `Permissions.kt`, `Errors.kt` | Deadlines, queues, IDs, ATT rules, UUIDs, name cutting, permission rules, error codes |

## Behavior

- Every command and every Bluetooth callback runs on one `HandlerThread`.
  `connectGatt` gets its handler; server, scan and advertise callbacks are posted
  to it.
- API 31+: `BLUETOOTH_SCAN` for `startScan`, `BLUETOOTH_ADVERTISE` for
  `startAdvertising` and `stopAdvertising`, `BLUETOOTH_CONNECT` for the other
  radio commands. API 30 and earlier: `ACCESS_FINE_LOCATION` for scanning, and
  nothing for the other roles. Cleanup commands run while the adapter is off.
- A connection requests an ATT MTU of 517 and reports `min(mtu - 3, 512)` as
  its value limits.
- A GATT procedure that passes its deadline in flight closes its connection
  and emits `connectionClosed`, so a late callback never answers a later one.
- A write without response also waits in the procedure queue: Android answers
  it with `onCharacteristicWrite` once the stack takes the value.
- `notify` sends an indication when the central enabled only indications.

## Known limits

- Android allows about five scan starts per 30 seconds per app. Past that, a
  scan silently reports nothing.
- On API 30 and earlier, scanning also needs the system location setting on.
- Whether the platform keeps a connectable advertisement running after a
  central connects depends on the device. The backend does not restart it.
