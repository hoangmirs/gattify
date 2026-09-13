# Windows

Target baseline: Windows 10 version 1903 (build 18362). The GATT client and
server APIs need Windows 10 1703 (build 15063); before 1903 an advertisement
carries no name. Windows 10 2004 (build 19041) and later also report the
address type and the scan response flag of each packet, and the advertising
data length of the adapter.

The Rust backend in `crates/tauri-plugin-gattify/src/winrt` implements every
command and event of `docs/native-bridge.md` on the WinRT Bluetooth APIs,
through the `windows` crate. It compiles for `x86_64-pc-windows-msvc`, and its
platform-neutral parts run as unit tests on every host. No radio behavior is
verified on hardware yet.

## Structure

| File | Responsibility |
| --- | --- |
| `backend.rs` | `WindowsBackend`, the engine thread and its current-thread runtime |
| `engine.rs` | Jobs, operations, deadlines, dispatch, `cancel`, `closeOwner`, `debugResources`, adapter changes |
| `adapter.rs` | The default adapter, its radio, readiness checks |
| `scanner.rs` | The one advertisement watcher and scan results |
| `central.rs` | Connections, the GATT procedure queue, discovery, reads, writes, subscriptions |
| `peripheral.rs` | Service providers, publication, the advertisement, requests, subscribers, notifications |
| `convert.rs` | Buffers, GUIDs, vectors, event handlers, `HRESULT` errors |
| `ids.rs`, `queue.rs`, `scan.rs`, `gatt.rs`, `advertise.rs`, `status.rs` | IDs, queues, packet merging, ATT rules, the name budget and publication steps, states, capabilities, error codes |

## Behavior

- One engine thread owns all state. Plugin setup creates it outside any
  runtime, so it runs its own current-thread tokio runtime. Commands and every
  WinRT callback post jobs to it; WinRT async operations run as tasks that post
  their results back. Nothing blocks the thread, and the event sink is called
  with no lock held. The thread lives as long as the process.
- A command that needs the adapter looks it up with
  `BluetoothAdapter.GetDefaultAsync` when none is known yet, and again while
  the known one is not on, so that a replaced adapter or a reinstalled driver
  counts. A new adapter replaces the old one and its radio handler. A lookup
  that takes longer than 5 s counts as no adapter. No adapter, or one
  without LE, is `unavailable`. `Radio.State` gives the state: `On` is `poweredOn`, `Off` and
  `Disabled` (a hardware switch or the firmware) are `poweredOff`, anything
  else is `unknown`. `Radio.StateChanged` drives `adapterStateChanged` and the
  sequence of the contract when Bluetooth turns off. The handler reads the
  state on the thread of the event, and the engine handles each change in
  that order, so a quick off and on both count. A watcher that stops on its
  own reads the radio first, so `adapterStateChanged` comes before its
  `scanStopped`.
- Windows gives no radio object to a process whose architecture differs from
  the system's, such as an x64 build under emulation on Windows on ARM. An LE
  adapter without a radio then counts as `poweredOn`, and a command fails with
  the error of its platform call, for example `bluetoothOff` from
  `RadioNotAvailable`. Later commands look for the radio again in the
  background; once it appears, its state applies.
- `checkPermissions` and `requestPermissions` report `notRequired` for every
  role and never show UI: an unpackaged desktop app has no runtime Bluetooth
  permission, and a packaged app declares the `bluetooth` capability in its
  manifest. A call that Windows denies rejects with `permissionDenied`.
- One `BluetoothLEAdvertisementWatcher` in active mode runs while any scan
  runs. A watcher that Windows aborts right after `Start` rejects `startScan`
  with the error its `Stopped` event reported. It has no platform filter: each packet goes to every scan whose filter
  matches its services, solicited services or service data. Windows reports an
  advertisement and its scan response as two packets, so the backend keeps the
  latest of each per device and reports them merged. The packets of a device
  not heard for 60 s are dropped, and at most 1024 cached names stay. A
  device ID stays with its Bluetooth address, which never leaves the backend.
- There is no connect call on Windows. `connect` opens the device with
  `FromBluetoothAddressAsync` (with the address type of the last scan result
  when Windows gave one), opens its `GattSession`, and sets
  `MaintainConnection`. A session that is not active at once gets a request
  for the cached services, whose answer closes unread: bleak reports adapters
  that connect only for a request (unverified here). It resolves when the
  session is active and `MaxPduSize`, the ATT MTU, exceeds 23, or 1 s after
  the session became active. The limits are `min(mtu - 3, 512)`.
- `disconnect` removes the value handlers, then closes every service, the
  device and the session, so that Windows can drop the link, and replies when
  the session reports the close or after 2 s. The `Close` calls run on a
  blocking thread, because a `Close` can hang. A session that closes, or a
  device that reports `Disconnected`, ends a connected link with
  `connectionClosed`.
- Each connection runs one GATT procedure at a time. A procedure that passes
  its deadline in flight closes the connection with `connectionClosed`. A
  cancelled one keeps its place until its completion, and a cancelled
  subscribe is rolled back before the next procedure. Discovery and reads
  bypass the Windows cache. A discovery that ends early, or whose link
  closes, stops before its next request and closes the services it holds,
  so that it cannot keep the link open. Reads and writes use the objects of
  the latest discovery, and a new discovery closes the service objects of
  older ones that no subscription uses. A characteristic handle is keyed by its ATT
  handle and UUID, so a second discovery returns the same handle.
- A read does not conflict with a subscription: Windows reports values
  through `ValueChanged` apart from reads. The value handler is registered
  before the descriptor write, and up to 256 values that arrive before the
  subscribe resolves are emitted right after its reply.
- `createServer` creates one `GattServiceProvider` per service and each
  characteristic with plain protection and no static value, so every read
  reaches the backend. `busy` rejects a service UUID that another live server
  registered.
- Server writes are answered in the order they arrived. `notify` sends with
  `NotifyValueForSubscribedClientAsync` to the one central and resolves when
  Windows completes it; one notification per server is outstanding. One that
  stays unanswered 2 s past its deadline is given up, and the next one goes.
- `SubscribedClientsChanged` is diffed into `subscriptionChanged`, with
  `maxValueLength = min(MaxNotificationSize, 512)`, and again when
  `MaxNotificationSizeChanged` fires. A central keeps its `central-<n>` for the
  process, keyed by the device ID of its session.
- Platform errors keep a native code: `bluetoothError<n>`,
  `gattCommunicationStatus<n>`, `gattProtocolError<n>` or `hresult0x<hex>`,
  also when they map to a contract code.

## Differences from Android and iOS

- Windows cannot advertise a local name of its own choosing. The name travels
  as service data for the service UUID, as on Android, with a budget of
  13 bytes: 31 legacy bytes minus 18 for the section header and the UUID.
  Windows places that data and reports `StartedWithoutAllAdvertisementData`
  when it does not fit. The start then reports `localNameIncluded: false` and
  `localNameTruncated: true`, or rejects with `payloadTooLarge` when the name
  is required. Whether the name fits next to a 128-bit UUID is unverified.
- A connectable, discoverable provider also advertises the computer name.
  Scanners report it as the local name, and the name rule of the contract
  prefers it, so other platforms see the computer name as `name`. The chosen
  name stays in `advertisement.serviceData`.
- Windows adds a service to its GATT database only while its provider
  publishes. A server's services become discoverable when the server first
  advertises; its other services are then published without advertising
  (`IsConnectable` false). A server that never advertises is never visible.
  Only one server of the process advertises at a time, so a second server
  stays invisible while another one advertises, and until it advertises
  itself. `stopAdvertising` publishes the advertised
  service again the same way, and new options restart its provider. A
  provider must stop before it publishes again, so the service leaves the
  database for a moment and connected centrals may lose it. The backend
  does not wait for Windows to report that: when a provider stops, each
  subscriber of its service gets `subscriptionChanged` with
  `subscribed: false`, and its queued notifications reject with
  `disconnected`. When the service is published again, the centrals that
  Windows still lists as subscribed count as subscribed again.
- A provider takes a new publication only after the previous one ended.
  The backend follows the `AdvertisementStatus` it reads after each start or
  stop, on each `AdvertisementStatusChanged`, and every second while a call
  waits, because Windows may not report a publication without an
  advertisement. Such a publication that shows no start after a second
  counts as started. An advertisement that shows none waits until its
  `startAdvertising` ends. Both are unverified.
- A service provider publishes primary services only. A service with
  `primary: false` is published as a primary service.
- Windows raises one `WriteRequested` per write, with an offset and no
  prepare or execute boundary. Each request counts as a whole write: a
  request at another offset than 0 fails with `invalidOffset`. Whether
  Windows assembles a long write into one request is unverified.
- A server sees no connection events: a central that leaves is noticed when
  it drops out of `SubscribedClients`. A peripheral cannot disconnect a
  central, as on iOS.
- `criticalStateLoss` reports `bluetoothOff`, or `unavailable` when the radio
  state becomes unknown. The Android reasons `notificationTimeout` and
  `serviceAddTimeout` do not occur: WinRT names each operation, so a late
  answer cannot reach the next one.
- `maxAdvertisingDataLength` is 31, or less when the adapter reports less.
  Extended advertising is not used. `simultaneousRoles` follows the peripheral
  role when the central role is supported.
- Windows may keep a physical link after `disconnect` while the system or
  another app uses the device.
- An advertisement that Windows stops on its own ends without an event, as on
  the other platforms.

## Verification

Compile-verified only: `cargo clippy -p tauri-plugin-gattify --target
x86_64-pc-windows-msvc` with all features and with no default features, and
`cargo check` of the lab app for that target. The unit tests of the
platform-neutral modules run with `cargo test` on any host. Nothing ran on
Windows or on radio hardware.
