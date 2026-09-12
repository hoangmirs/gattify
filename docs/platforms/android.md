# Android

Target baseline: API 26+, compile SDK 36, Java 17. The Kotlin backend in
`crates/tauri-plugin-gattify/android` implements every command and event of
`docs/native-bridge.md`. It compiles in CI, and its pure parts run as JUnit
tests. No radio behavior is verified on hardware yet.

## Structure

| File | Responsibility |
| --- | --- |
| `GattifyPlugin.kt` | Tauri commands `execute` and `setEventChannel`, permission aliases and the permission callback, the backend lifecycle |
| `Backend.kt` | The handler thread, dispatch, readiness checks, owners, events, `cancel`, `closeOwner`, adapter broadcasts, dispose |
| `Central.kt` | Connections, the MTU request, discovery, reads, writes, subscriptions, one GATT procedure at a time per connection |
| `Scanner.kt` | Logical scans on one shared platform scan, throttled results, timeouts |
| `Peripheral.kt` | The shared `BluetoothGattServer`, stored values, CCCDs, long writes, notification flow control |
| `Advertiser.kt` | The one legacy advertisement, with the name as service data in the scan response |
| `Wire.kt`, `Payloads.kt`, `ExecuteResult.kt` | JSON replies, events and command payloads |
| `Procedures.kt`, `Queues.kt`, `ScanPlan.kt`, `Ids.kt`, `Gatt.kt`, `Uuids.kt`, `Names.kt`, `Permissions.kt`, `Errors.kt` | Deadlines, queues, the scan restart plan, IDs, ATT rules, UUIDs, name cutting, permission rules, error codes |

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
  A cancelled subscribe keeps its characteristic reserved until its rollback
  has run, so the rollback never disables a later subscription.
- A write without response also waits in the procedure queue: Android answers
  it with `onCharacteristicWrite` once the stack takes the value.
- A failed `unsubscribe` rejects with the platform code and keeps the
  subscription. When notifications cannot be switched back on locally, the
  connection closes instead.
- Every `startScan` shares one platform scan with the union of the filters, and
  each result goes to every scan whose filter matches. Changes within 100 ms
  merge into one restart, a narrower filter never restarts the scan, and at most
  four starts happen in 31 s. A wider filter past that limit waits.
- A CCCD write must be two bytes, with no reserved bit and only the modes that
  the characteristic has. `notify` sends an indication when the central enabled
  only indications.
- The server callbacks carry no request ID. When an `addService` or a
  notification stays unanswered past its deadline, the platform server closes:
  its pending calls reject, subscribers end with `subscribed: false`, and its
  servers report `criticalStateLoss` with the reason `serviceAddTimeout` or
  `notificationTimeout`. The next `createServer` opens a new one.
- When the activity is destroyed, the backend releases every resource without
  events, unregisters its receiver and quits its thread. The next command creates
  a new backend. IDs belong to the process, so none repeats.

## Known limits

- Android counts scan starts per app, so another scanner in the same app still
  uses up the same quota.
- On API 30 and earlier, scanning also needs the system location setting on.
- Whether the platform keeps a connectable advertisement running after a
  central connects depends on the device. The backend does not restart it.
