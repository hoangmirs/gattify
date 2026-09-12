# Native bridge contract v1

This document is the contract between the Rust plugin and the native layer:
Kotlin on Android and Swift on iOS. Both platforms implement it exactly. The
Rust types in `crates/tauri-plugin-gattify/src/backend.rs` and `src/model.rs`
are the source of truth for every JSON shape below.

The native layer does raw GATT only. It knows nothing about the peer protocol.

## Transport

Rust calls two native commands through the Tauri mobile plugin API.

| Command | Arguments | Result |
| --- | --- | --- |
| `execute` | `{ operationId, ownerId, deadlineMillis, command }` | resolve with a Reply, or reject with a message and an error code |
| `setEventChannel` | `{ channel }` | resolve with nothing |

`command` is `{ "kind": "<camelCase>", "payload": ... }`. A command without a
payload has no `payload` key. A Reply has the same shape: `{ "kind": "empty" }`
or `{ "kind": "scanStarted", "payload": { "scanId": "scan-1" } }`.

Rust sends `setEventChannel` once, during plugin setup. The native layer keeps
the latest channel. Before a channel exists, the native layer drops events.

The native layer sends each event through the channel as an envelope:

```json
{ "ownerId": "webview:main", "event": { "kind": "scanResult", "payload": { "device": {} } } }
```

Send only valid JSON. The Android receiver in Tauri unwraps the parse result,
so invalid JSON crashes the app.

A rejection carries a `code`. Use an error code string from the table below
when one fits. Otherwise use a platform code: `gattStatus<n>` for an Android
GATT status, `cbError<n>` for an iOS `CBError`, and `cbAttError<n>` for an iOS
`CBATTError`. Rust turns an unknown code into `internal` and keeps the platform
code as `nativeCode`.

| Code | Use |
| --- | --- |
| `permissionDenied` | The OS permission for the role is missing |
| `bluetoothOff` | The adapter is off |
| `unavailable` | The device has no BLE adapter, or the adapter is resetting or in an unknown state after the wait |
| `unsupported` | The device cannot do this, for example no advertiser |
| `invalidArgument` | A malformed payload: bad base64, an unknown characteristic key, a service UUID the server does not have |
| `invalidHandle` | An ID that is unknown, stale, or owned by another owner |
| `busy` | The resource is in use: a second advertisement, a second connection to one device |
| `timeout` | The deadline passed |
| `disconnected` | The link closed during the procedure |
| `payloadTooLarge` | A value is longer than the limit |
| `cancelled` | A `cancel` or `closeOwner` ended the procedure |
| `internal` | Anything else |

## Threading

Tauri calls `execute` on its own thread: a serial `ipc` dispatch queue on iOS,
a Rust thread through JNI on Android. BLE callbacks arrive on other threads.

- iOS: create one serial `DispatchQueue` for the plugin. Pass it to
  `CBCentralManager(delegate:queue:)` and `CBPeripheralManager(delegate:queue:)`.
  Move each command onto that queue before touching state.
- Android: run all state changes on one `HandlerThread`. Post each command and
  each GATT callback to its handler.

`Invoke.resolve` and `Invoke.reject` may run on any thread.

## Identifiers

The native layer allocates every ID as an opaque string. An ID is unique for the
process and never reused.

| ID | Form | Notes |
| --- | --- | --- |
| device | `device-<n>` | One per remote peripheral: `CBPeripheral.identifier` on iOS, the Bluetooth address on Android. The same remote keeps its ID for the process. Never expose the address or the CoreBluetooth UUID |
| scan | `scan-<n>` | |
| connection | `connection-<n>` | |
| subscription | `subscription-<n>` | |
| server | `server-<n>` | |
| central | `central-<n>` | One per remote central that talks to a local server. Used as `peerId` in server commands and events |
| service handle | `<connectionId>/service-<n>` | Valid only on that connection |
| characteristic handle | `<connectionId>/characteristic-<n>` | Valid only on that connection. A second discovery returns the same handle for the same characteristic |

A server names a characteristic with a characteristic key:
`<serviceInstanceKey>/<characteristicInstanceKey>`, from the `ServerDefinition`.

## Owners

Every resource belongs to the `ownerId` that created it. A command that names a
resource of another owner rejects with `invalidHandle`. Events about a resource
go to its owner.

The owner family is the text after the first `:` of the owner ID. `webview:main`
and `gattify-peer:main` both have the family `main`. The peer driver dials
devices that its webview scanned, so connect accepts a device ID when any owner
of the same family received it in a `scanResult`.

## UUIDs

Rust sends every UUID as a lowercase 128-bit string with hyphens. The native
layer sends every UUID the same way. iOS reports a 16-bit or 32-bit `CBUUID`
as `180D` or `0000180D`. Expand it with the Bluetooth base UUID
`0000xxxx-0000-1000-8000-00805f9b34fb`, then lowercase it.

## Deadlines

`deadlineMillis` is a relative timeout in milliseconds, or null. When a
procedure is still pending at the deadline, reject it with `timeout` and clean
it up. A timed-out connect cancels the connection attempt.

When `deadlineMillis` is null, use these defaults:

| Command | Default |
| --- | --- |
| `connect` | `options.timeoutMs`, else 15000 |
| `discoverServices` | 10000 |
| `read`, `write`, `subscribe`, `unsubscribe`, `notify` | 5000 |
| `createServer`, `startAdvertising` | 5000 |
| `requestPermissions` | none: the user answers a system prompt |

## Adapter readiness

iOS creates `CBCentralManager` on the first command that needs the central role,
and `CBPeripheralManager` on the first command that needs the peripheral role.
Never create a manager in `load`: a manager shows the Bluetooth prompt. A
command that creates a manager waits for the first `didUpdateState`, within its
deadline. Then, if the state is not `poweredOn`, the command rejects:

| State | Code |
| --- | --- |
| `poweredOff` | `bluetoothOff` |
| `unauthorized` | `permissionDenied` |
| `unsupported` | `unavailable` |
| `resetting`, `unknown` | `unavailable` |

Android rejects with `unavailable` when there is no adapter, `bluetoothOff`
when the adapter is off, and `permissionDenied` when the runtime permission for
the role is missing.

## Commands

### getState

Reply: `{ "kind": "state", "payload": "<AdapterState>" }`. The states are
`unknown`, `unavailable`, `unauthorized`, `poweredOff`, `resetting`, `poweredOn`.

- iOS: when a manager exists, map its state. Otherwise read
  `CBManager.authorization`: `denied` and `restricted` give `unauthorized`, and
  anything else gives `unknown`. Do not create a manager.
- Android: no adapter gives `unavailable`. On API 31 and later, a missing
  `BLUETOOTH_CONNECT` gives `unauthorized`. Then an enabled adapter gives
  `poweredOn`, else `poweredOff`.

### getCapabilities

Reply: `{ "kind": "capabilities", "payload": Capabilities }`, with every field
present. A `Support` is `{ "level", "reason", "description": null }`. A
supported capability uses the reason `available`.

| Field | iOS | Android |
| --- | --- | --- |
| `central` | supported | supported |
| `peripheral` | supported | supported when `adapter.bluetoothLeAdvertiser` is not null, else unsupported with `noAdvertiser` |
| `advertising` | supported | as `peripheral` |
| `targetedNotify` | supported | as `peripheral` |
| `simultaneousRoles` | supported | as `peripheral` |
| `background` | unsupported, `foregroundOnlyContract` | unsupported, `foregroundOnlyContract` |
| `maxConnections` | null | null |
| `maxAdvertisingDataLength` | 28 | 31 |

With no adapter, Android reports every capability except `background` as
unsupported with the reason `noAdapter`.

### checkPermissions

Reply: `{ "kind": "permissions", "payload": { "scan", "connect", "advertise" } }`.
Each value is `granted`, `promptable`, `deniedPermanently`, `restricted`,
`notRequired` or `unknown`.

- iOS: one authorization covers all three roles. `CBManager.authorization`
  `allowedAlways` gives `granted`, `notDetermined` gives `promptable`, `denied`
  gives `deniedPermanently` and `restricted` gives `restricted`.
- Android, API 31 and later: `scan` is `BLUETOOTH_SCAN`, `connect` is
  `BLUETOOTH_CONNECT`, `advertise` is `BLUETOOTH_ADVERTISE`. A granted
  permission gives `granted`. A permission that is not granted gives
  `promptable`, or `deniedPermanently` after a request was denied and
  `shouldShowRequestPermissionRationale` returns false.
- Android, API 30 and earlier: `scan` follows `ACCESS_FINE_LOCATION`. `connect`
  and `advertise` are `notRequired`.

### requestPermissions

Payload: `{ "scan": bool, "connect": bool, "advertise": bool }`. Reply: as
`checkPermissions`, after the user answers.

- iOS: when the authorization is `notDetermined`, create the central manager.
  This shows the system prompt. Reply when `didUpdateState` reports a state
  other than `unknown`. Otherwise reply at once.
- Android: request the runtime permissions of the requested roles through the
  Tauri permission API, then reply with the new state. Request nothing when
  every requested role is `granted` or `notRequired`.

### startScan

Payload: `{ "serviceUuids": [string], "timeoutMs": number | null }`.
Reply: `{ "kind": "scanStarted", "payload": { "scanId" } }`.

- Rust rejects an empty filter from a webview. The native layer treats an empty
  filter as "every device".
- For each advertisement that matches a filter UUID, emit `scanResult` to the
  scan owner. Emit at most one result per device per scan every 1000 ms.
- Several scans can run at the same time. iOS runs one scan with the union of
  the filters and routes each result to every scan whose filter matches.
- After `timeoutMs`, stop the scan and emit `scanStopped`.
- On Android, use `SCAN_MODE_LOW_LATENCY`.

`DiscoveredDevice`:

```json
{
  "id": "device-3",
  "name": "Host A",
  "rssi": -58,
  "serviceUuids": ["80ff87c3-8e84-4914-aedc-0d6a3ba5534d"],
  "advertisement": {
    "localName": "Host A",
    "serviceData": [{ "serviceUuid": "80ff87c3-8e84-4914-aedc-0d6a3ba5534d", "bytesBase64": "SG9zdCBB" }],
    "manufacturerData": [{ "companyId": 76, "bytesBase64": "AQI=" }],
    "connectable": true
  },
  "observedAtMillis": 1757664000000,
  "scanId": "scan-1"
}
```

`name` is the advertised local name, else the cached device name, else null.
`observedAtMillis` is wall-clock time in milliseconds since the Unix epoch.

### stopScan

Payload: `{ "scanId" }`. Reply: `empty`. Emit no `scanStopped`.

### connect

Payload: `{ "deviceId", "options": { "timeoutMs": number | null } }`.
Reply: `{ "kind": "connected", "payload": { "connectionId", "limits": LinkLimits } }`.

- Reject with `invalidHandle` when the device ID is unknown, or when no owner of
  the caller's family received it in a scan result.
- Reject with `busy` when the device already has a connection.
- Android: `connectGatt(context, false, callback, TRANSPORT_LE)`. On
  `STATE_CONNECTED`, call `requestMtu(517)`. Resolve on `onMtuChanged`. When the
  MTU request fails, or no answer arrives within 5 s, resolve with an MTU of 23.
- iOS: `connect(peripheral)`. Resolve on `didConnect`.

`LinkLimits`:

```json
{ "writeWithResponse": 182, "writeWithoutResponse": 182, "notification": 182, "attMtu": 185 }
```

`writeWithResponse` and `writeWithoutResponse` are the largest value that one
ATT Write Request or Write Command carries: the ATT MTU minus 3. They are never
the long-write maximum. On iOS use
`peripheral.maximumWriteValueLength(for: .withoutResponse)` for all three
lengths, and that value plus 3 as `attMtu`. On Android use the MTU minus 3.

### disconnect

Payload: `{ "connectionId" }`. Reply: `empty`, after the link closes or after
2 s. End the subscriptions of the connection without events. Reject its pending
procedures with `disconnected`. Emit no `connectionClosed`.

### discoverServices

Payload: `{ "connectionId" }`. Reply: `{ "kind": "services", "payload": [ServiceInstance] }`.

```json
[{
  "handle": "connection-2/service-1",
  "uuid": "80ff87c3-8e84-4914-aedc-0d6a3ba5534d",
  "characteristics": [{
    "handle": "connection-2/characteristic-1",
    "uuid": "b1e10f10-6a2c-4a62-8e9e-2c938fa30101",
    "properties": { "read": true, "write": false, "writeWithoutResponse": false, "notify": false, "indicate": false }
  }]
}]
```

iOS discovers all services, then the characteristics of each service, and
replies when every characteristic discovery finishes.

### read

Payload: `{ "connectionId", "characteristic" }`.
Reply: `{ "kind": "bytes", "payload": { "valueBase64" } }`.

iOS reports a read and a notification through the same callback. When a read
is pending on a characteristic, the next value for it answers the read.

### write

Payload: `{ "connectionId", "characteristic", "valueBase64", "writeType" }`,
where `writeType` is `withResponse` or `withoutResponse`. Reply: `empty`.

- `withResponse` resolves after the write response.
- `withoutResponse` resolves when the stack accepts the value. On iOS, when
  `canSendWriteWithoutResponse` is false, wait for `peripheralIsReady`.
- A value longer than 512 bytes rejects with `payloadTooLarge`.

### subscribe

Payload: `{ "connectionId", "characteristic" }`.
Reply: `{ "kind": "subscriptionStarted", "payload": { "subscriptionId" } }`.

- Enable notifications, or indications when the characteristic has only
  `indicate`. Android calls `setCharacteristicNotification` and writes the
  client configuration descriptor `00002902-0000-1000-8000-00805f9b34fb`. iOS
  calls `setNotifyValue(true, for:)`.
- Resolve after the descriptor write succeeds: `onDescriptorWrite` on Android,
  `didUpdateNotificationStateFor` on iOS.
- A second subscription to the same characteristic on one connection rejects
  with `busy`.
- Emit `characteristicValue` for each value to the subscription owner.

### unsubscribe

Payload: `{ "subscriptionId" }`. Reply: `empty`. Disable notifications. Emit
nothing more for the subscription.

### createServer

Payload: a `ServerDefinition`. Reply: `{ "kind": "serverCreated", "payload": { "serverId" } }`.

```json
{
  "services": [{
    "instanceKey": "peer",
    "uuid": "80ff87c3-8e84-4914-aedc-0d6a3ba5534d",
    "primary": true,
    "characteristics": [{
      "instanceKey": "info",
      "uuid": "b1e10f10-6a2c-4a62-8e9e-2c938fa30101",
      "properties": { "read": true, "write": false, "writeWithoutResponse": false, "notify": false, "indicate": false },
      "initialValueBase64": "AQ==",
      "maxValueLength": 1
    }]
  }]
}
```

- Map `read`, `write`, `writeWithoutResponse`, `notify` and `indicate` to the
  platform properties and permissions. Android adds the client configuration
  descriptor to every characteristic with `notify` or `indicate`. iOS adds it
  itself.
- Keep every value in the native layer. iOS creates each characteristic with a
  nil value, so that iOS asks the plugin for every read.
- Resolve after the stack adds every service: each `didAdd` on iOS, each
  `onServiceAdded` on Android. Android adds one service at a time.
- Reject with `busy` when another server already registered a service UUID of
  the definition.

### closeServer

Payload: `{ "serverId" }`. Reply: `empty`. Stop its advertisement, remove its
services and forget its subscribers. Emit nothing.

### startAdvertising

Payload: `{ "serverId", "options": { "serviceUuid", "localName", "localNameOptional" } }`.
Reply: `{ "kind": "advertisingStarted", "payload": { "localNameIncluded", "localNameTruncated" } }`.

- Reject with `invalidArgument` when the server has no service with that UUID.
- One advertisement exists per process. When another server advertises, reject
  with `busy`. When the same server advertises, restart with the new options.
- Cut the local name at a UTF-8 character boundary to the platform budget:
  28 bytes on iOS, 13 bytes on Android. When the name is cut and
  `localNameOptional` is false, reject with `payloadTooLarge`.
- iOS: `startAdvertising` with `CBAdvertisementDataServiceUUIDsKey` and
  `CBAdvertisementDataLocalNameKey`. Resolve on `didStartAdvertising`.
- Android: legacy advertising, `ADVERTISE_MODE_LOW_LATENCY`, connectable, no
  timeout, `ADVERTISE_TX_POWER_HIGH`. The advertisement holds the service UUID.
  `setIncludeDeviceName` stays false, because it sends the system device name.
  The scan response holds the local name as service data for the service UUID.
  Resolve on `onStartSuccess`. Map `onStartFailure`:
  `ADVERTISE_FAILED_DATA_TOO_LARGE` to `payloadTooLarge`,
  `ADVERTISE_FAILED_TOO_MANY_ADVERTISERS` and `ADVERTISE_FAILED_ALREADY_STARTED`
  to `busy`, `ADVERTISE_FAILED_FEATURE_UNSUPPORTED` to `unsupported`, and any
  other code to `internal`.

`localNameIncluded` is true when a name with at least one byte is advertised.
`localNameTruncated` is true when the name was cut.

### stopAdvertising

Payload: `{ "serverId" }`. Reply: `empty`.

### setValue

Payload: `{ "serverId", "characteristicKey", "valueBase64" }`. Reply: `empty`.
Store the value for reads. A value longer than `maxValueLength` rejects with
`payloadTooLarge`. Do not notify.

### notify

Payload: `{ "serverId", "peerId", "characteristicKey", "valueBase64" }`.
Reply: `empty`.

- Reject with `invalidHandle` when the central has no subscription to the
  characteristic.
- Reject with `payloadTooLarge` when the value is longer than the notification
  size of that central: the `maxValueLength` of its last `subscriptionChanged`.
- Resolve only after the stack accepts the value. This is the flow control for
  the host.
  - iOS: `updateValue(_:for:onSubscribedCentrals: [central])`. When it returns
    false, keep the value in a FIFO queue and retry on
    `peripheralManagerIsReady(toUpdateSubscribers:)`.
  - Android: `notifyCharacteristicChanged(device, characteristic, false, value)`
    on API 33 and later, else set the value and call the older overload. Resolve
    on `onNotificationSent`. Reject when its status is not `GATT_SUCCESS`. Keep
    one notification outstanding for the whole server, and queue the others.
- Do not change the stored read value.

### cancel

Payload: `{ "operationId" }`. Reply: `empty`. When a procedure with that
operation ID is pending, reject it with `cancelled` and stop it: a connect
cancels the connection attempt. Rust has already checked that the procedure
belongs to the caller. An unknown operation ID is not an error.

### closeOwner

Reply: `empty`. Release everything the owner holds: stop its scans, disconnect
its connections, end its subscriptions, stop its advertisement and close its
servers. Reject its pending procedures with `cancelled`. Emit no events for the
released resources. `closeOwner` is idempotent.

### debugResources

Reply: `{ "kind": "resources", "payload": { "scans", "connections", "subscriptions", "servers" } }`,
counting the resources of the caller.

## Events

Every event goes to the owner of its resource.

| Kind | Payload | When |
| --- | --- | --- |
| `scanResult` | `{ "device": DiscoveredDevice }` | An advertisement matches a scan |
| `scanStopped` | `{ "scanId" }` | A scan ends for any reason other than `stopScan` or `closeOwner` |
| `connectionClosed` | `{ "connectionId" }` | A link closes for any reason other than `disconnect` or `closeOwner`. Its handles and subscriptions become stale |
| `characteristicValue` | `{ "subscriptionId", "valueBase64" }` | A notification or indication arrives |
| `serverWrite` | `{ "serverId", "peerId", "characteristicKey", "valueBase64" }` | A central writes a characteristic |
| `subscriptionChanged` | `{ "serverId", "peerId", "characteristicKey", "subscribed", "maxValueLength" }` | A central subscribes or unsubscribes |
| `adapterStateChanged` | `{ "state" }` | The adapter state changes. Send it to every owner that holds a resource |
| `criticalStateLoss` | `{ "resourceId", "reason" }` | A server loses its registration, for example when Bluetooth turns off. `resourceId` is the server ID |

### Server writes

When a central writes a characteristic with `write` or `writeWithoutResponse`:

1. Reject a value longer than `maxValueLength` with the ATT error
   `invalidAttributeValueLength`, and emit nothing.
2. Answer the write when it needs a response.
3. Emit `serverWrite`. Keep the order in which the writes arrived.

A write does not change the stored read value. Android buffers a prepared write
for each central and emits it once, on `onExecuteWrite` with `execute` true.

### Subscriptions

When a central enables notifications or indications on a characteristic, emit
`subscriptionChanged` with `subscribed: true` and `maxValueLength` set to the
notification size for that central: `central.maximumUpdateValueLength` on iOS,
the MTU of that central minus 3 on Android, and 20 before an MTU exchange.
When the central disables them, or disconnects, emit `subscribed: false` with
`maxValueLength: null`. When the MTU of a subscribed central changes on
Android, emit `subscribed: true` again with the new size.

## Rust side

- Rust checks roles and the service UUID scope before `execute`. See the scope
  section of `docs/superpowers/specs/2026-09-10-gattify-design.md`.
- Events for an owner that starts with `gattify-peer:` go to the peer driver.
  Rust emits every other event only to the owner webview, as
  `gattify://<kind in kebab case>` with the event payload. For example,
  `scanResult` becomes `gattify://scan-result` with the payload `{ "device": {...} }`.
