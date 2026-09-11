# gattify design

Date: 2026-09-10
Status: approved design. Each sub-project below gets its own implementation plan.

## Goal

Release this repository as a public Tauri plugin named gattify. An app uses gattify to exchange messages between two phones over BLE, with no network. A host phone advertises. A joiner phone scans for the host, connects, and exchanges complete messages with the host.

## Context

gattify needs both BLE roles. The host uses the peripheral role, because it advertises. The joiner uses the central role, because it scans.

gattify uses the MIT license, and `deny.toml` allows only permissive licenses for dependencies. A closed app can therefore ship gattify.

The repository has these parts today:

- DTOs, the error contract, the owner-scoped `Manager`, the `Backend` trait and a mock backend.
- The peer protocol: frames, `Sender` and `Receiver`.
- A TypeScript facade with raw GATT handles and peer handles.
- Kotlin and Swift files that report adapter state only. `init()` does not register them.

The repository has no native radio code. It has no event delivery: the `Event` enum exists, but no code sends an event to Rust or to the webview.

## Decisions

| Topic | Decision | Reason |
| --- | --- | --- |
| Name | gattify. Crate `tauri-plugin-gattify`, npm package `tauri-plugin-gattify-api` | `tauri-plugin-ble` and `tauri-plugin-bluetooth` are taken on crates.io |
| Crates | One crate. `ble-core` and `ble-peer` become modules of `tauri-plugin-gattify` | gattify is a Tauri product. One crate is one package to publish and version |
| Native code | Kotlin on Android and Swift on iOS, through the Tauri mobile plugin API | This is the Tauri convention for mobile plugins |
| Native responsibility | Raw GATT only | The peer protocol exists once, in Rust. The mock tests cover it, and Android and iOS cannot disagree about frames |
| Platforms | Android and iOS. Desktop builds keep the `Unsupported` backend | The first release targets two phones near each other |
| Scope | A Tauri global scope lists the service UUIDs that an app allows | A script in the webview can reach only the services that the app lists |
| Release | Public on crates.io and npm. The owner runs every publish command | Apps depend on published versions |

These decisions replace two parts of ADR-001: the three-crate layout and the plan for desktop backends. Sub-project 1 records them in ADR-002.

## Out of scope

- Desktop backends for macOS, Windows and Linux.
- Background operation. The contract stays foreground-only.
- Encryption and authentication, mesh routing, and L2CAP.
- Publish commands. The owner runs `cargo publish` and `npm publish`.

## Terms

| Term | Meaning |
| --- | --- |
| host | The device that runs the peer GATT server and advertises. It uses the peripheral role |
| joiner | The device that scans and dials a host. It uses the central role |
| native layer | The Kotlin and Swift code |
| mobile backend | The Rust `Backend` that sends each command to the native layer |
| peer driver | The Rust code that runs the peer protocol over GATT commands and events |
| owner | The creator of a resource, as an `OwnerId`. The owner is a webview, or the peer driver of a webview |
| scope | The list of service UUIDs that an app allows |
| lab app | The example Tauri app in `examples/gattify-lab` |

## Architecture

```
WebView      tauri-plugin-gattify-api
  | invoke plugin:gattify|...           ^ gattify:// events, to the owner webview only
Rust         commands -> role check -> scope check -> Manager -> Backend
                                                      peer driver (Sender, Receiver per peer)
  | run_mobile_plugin("execute")        ^ ipc::Channel with event envelopes
Native       Kotlin (Android), Swift (iOS): raw GATT only
```

### Crate layout

```
crates/tauri-plugin-gattify/
  build.rs            tauri_plugin::Builder: permissions, android and ios paths
  src/lib.rs          init(), BleRuntime, feature gates
  src/core/           from ble-core: DTOs, errors, Manager, Backend, mock
  src/peer/           from ble-peer: frame, sender, receiver, driver
  src/mobile.rs       mobile backend, compiled for android and ios only
  src/commands.rs     Tauri commands, role checks, scope checks
  src/events.rs       event router and webview cleanup
  android/  ios/  permissions/
packages/plugin-gattify/
examples/gattify-lab/
fuzz/
```

The `tauri` feature is on by default. It enables commands, the mobile backend and the event router. The `mock` feature exports `MockBackend`. Unit tests and the fuzz target build without the `tauri` feature.

### Rename map

| Old | New |
| --- | --- |
| crate `tauri-plugin-ble` | crate `tauri-plugin-gattify` |
| crates `ble-core`, `ble-peer` | modules `core`, `peer` |
| `Builder::new("ble")` | `Builder::new("gattify")` |
| `plugin:ble\|<command>` | `plugin:gattify\|<command>` |
| permissions `ble:scan`, `ble:peer`, and the others | `gattify:scan`, `gattify:peer`, and the others |
| events `ble://<name>` | `gattify://<name>` |
| npm `tauri-plugin-ble-api` in `packages/plugin-ble` | npm `tauri-plugin-gattify-api` in `packages/plugin-gattify` |
| `dev.taurible.plugin.BlePlugin` | `dev.gattify.plugin.GattifyPlugin` |
| Swift `BlePlugin`, `init_plugin_ble` | Swift `GattifyPlugin`, `init_plugin_gattify` |
| `repository = "https://github.com/hoangmirs/tauri-ble"` | `repository = "https://github.com/hoangmirs/gattify"` |

Names that describe the technology stay: `BleError`, `BleResult`, `BleRuntime`, `createBle()`, and the characteristic UUIDs in `docs/protocol.md`.

## Contracts

### 1. Backend and event sink

```rust
pub type EventSink = Arc<dyn Fn(OwnerId, Event) + Send + Sync>;

#[async_trait]
pub trait Backend: Send + Sync + 'static {
    async fn execute(&self, context: OperationContext, command: Command) -> BleResult<Reply>;
}
```

Each backend receives an `EventSink` when it is constructed. The `Backend` trait does not change.

`Event::SubscriptionChanged` gets one new field, `max_value_length: Option<u32>`. A host uses this field as the notification size for that central.

The mock gains `MockAir`, which links two `MockBackend` values. A write on one backend arrives as an event on the other. A test can tell `MockAir` to drop a frame or to disconnect a link.

### 2. Native bridge

The native layer exposes two commands to Rust:

| Native command | Arguments | Result |
| --- | --- | --- |
| `execute` | `{ operationId, ownerId, deadlineMillis, command }` | A `Reply` as JSON, or a rejection with `{ code, message }`. `code` is an `ErrorCode` string |
| `setEventChannel` | `{ channel }` | Empty |

During plugin setup, Rust creates the channel with `tauri::ipc::Channel::new` and sends it with `setEventChannel`. Tauri 2.11 registers a channel from `Channel::new` for mobile, and a native `channel.send` calls the Rust closure. The native layer sends only valid JSON, because the Tauri Android receiver unwraps the parse result.

Each event is an envelope:

```json
{ "ownerId": "webview:main", "event": { "kind": "scanResult", "payload": {} } }
```

The native layer obeys these rules:

- Serialize GATT procedures for each connection. Let independent connections run at the same time.
- Resolve `Notify` only after the stack accepts the value. On Android, wait for `onNotificationSent`. On iOS, wait until `updateValue` returns true, or for `peripheralManagerIsReady`. This is the flow control for the host.
- Connect only to a `DeviceId` that the same owner received in a scan result.
- Release every resource of an owner on `CloseOwner`.
- On Android, request an MTU of 517 after a connection starts. Report the result in the `LinkLimits` of `Connected`.
- Report the notification size of each central in `SubscriptionChanged`.
- On iOS, create the CoreBluetooth managers on first use, not in `load`. A manager at load time shows the Bluetooth prompt when the app starts.

### 3. Event router

Rust receives each envelope and routes it:

1. If the owner starts with `gattify-peer:`, Rust sends the event to the peer driver.
2. Otherwise, Rust emits the event only to the owner webview, as `gattify://<kind>` in kebab case. For example, `scanResult` becomes `gattify://scan-result`.

When a webview starts a page load or closes, Rust runs `CloseOwner` for the webview owner and for its peer owner. A reload then cannot leave a scan or an advertisement active.

### 4. Peer driver

The peer driver runs its commands under the owner `gattify-peer:<webview label>`.

These protocol details are new in v1. Sub-project 2 adds them to `docs/protocol.md`:

- The Info characteristic value is one byte: the protocol major version, `0x01`.
- HELLO and HELLO_ACK carry no payload.
- CLOSE ends a peer. The receiver of CLOSE does not reply.

`createEndpoint` gets a new option, `listen`, which is false by default. A host sets `listen: true`. A joiner leaves it false, so the joiner does not advertise.

**Host flow:**

1. `create_endpoint` checks the service UUID against the scope.
2. The driver runs `CreateServer` with one service: Info (read), RX (write with response) and TX (notify).
3. The driver runs `StartAdvertising` with the service UUID and the optional local name.
4. On `SubscriptionChanged` for TX, the driver stores the notification size of that central.
5. On a HELLO in RX from a central with no peer, the driver creates a peer. It sends HELLO_ACK to that central with `Notify` and emits `gattify://peer-ready`.
6. On a HELLO from a central that has a peer, the driver closes the old peer with reason `lost`. Then it creates a new peer as in step 5.
7. The driver ignores any other frame from a central with no peer.
8. On a DATA frame in RX from a peer, the driver passes the frame to that peer's `Receiver`. For each complete message, it emits `gattify://peer-message`. It sends the ACK frame with `Notify`.
9. The driver sends each outgoing frame with `Notify` to that central, one frame at a time.
10. On `SubscriptionChanged` with `subscribed: false`, or on CLOSE, the driver closes the peer.

**Joiner flow:**

1. `dial_peer` uses the service UUID of its endpoint. `create_endpoint` already checked that UUID against the scope.
2. The driver runs `Connect`, then `DiscoverServices`.
3. The driver finds the endpoint service and its Info, RX and TX characteristics. It reads Info and requires the value `0x01`.
4. The driver subscribes to TX, then writes HELLO to RX with a response.
5. On HELLO_ACK, `dial` resolves with the peer.
6. The driver sends each outgoing frame with a write to RX with a response, one frame at a time.
7. On a DATA frame from TX, the driver passes the frame to the `Receiver`. For each complete message, it emits `gattify://peer-message`. It sends the ACK frame with a write to RX.
8. On `ConnectionClosed`, or on CLOSE, the driver closes the peer.
9. If HELLO_ACK does not arrive within 10 seconds, `dial` fails with `Timeout` and the driver disconnects.

**Frame size.** The joiner uses `LinkLimits.write_with_response`. The host uses the notification size from step 4 of the host flow. Both use 20 bytes when the native layer reports no size.

**Send.** `send_peer` gives the payload to the peer's `Sender`. A timer calls `Sender::poll` every 100 ms for each peer with a pending message. On an ACK, `send_peer` resolves with `{ messageId, delivery: "transportAcknowledged" }`. On a failure, `send_peer` rejects with a `BleError` that carries a `delivery` value.

**Close.** `close_peer` sends CLOSE and does not wait for delivery. A joiner then disconnects. A host forgets the peer, because iOS gives a peripheral no way to disconnect a central. `close_endpoint` closes every peer, stops the advertisement and closes the server. For each closed peer, Rust emits `gattify://peer-closed` with `{ peerId, reason }`, where `reason` is `local`, `remote` or `lost`.

**Limits.** The defaults in `docs/protocol.md` apply: 16 KiB for each message, 5 s for each ACK, two retransmissions and a 30 s absolute deadline.

### 5. TypeScript API changes

- `PeerOptions` gets `listen?: boolean`, which is false by default.
- `Peer` gets `onClose(callback: (reason: "local" | "remote" | "lost") => void): () => void`.
- Every invoke name and event name uses `gattify`.

The README shows how an app wraps a `Peer` as a link with `send`, `onFrame`, `onClose` and `close`.

### 6. Scope

gattify adds the permission `gattify:scope`. This permission holds no commands. An app lists its service UUIDs in its capability file:

```json
{ "identifier": "gattify:scope", "allow": [{ "serviceUuid": "6e400001-b5a3-f393-e0a9-e50e24dcca9e" }] }
```

Each command reads the scope with `GlobalScope`. An empty scope rejects every radio command. Status commands and cleanup commands do not read the scope.

| Command | Rule |
| --- | --- |
| `StartScan` | The filter is not empty, and every UUID in the filter is in the scope |
| `Connect` | The native layer connects only to scanned devices. See the native bridge rules |
| `DiscoverServices` | Rust removes every service outside the scope from the reply |
| `Read`, `Write`, `Subscribe` | The characteristic handle came from a filtered discovery on the same connection |
| `CreateServer` | Every service UUID is in the scope |
| `StartAdvertising` | The service UUID is in the scope |
| `create_endpoint` | The endpoint service UUID is in the scope |

Rust compares UUIDs in lowercase 128-bit form. It expands a 16-bit or 32-bit UUID with the Bluetooth base UUID first. Rust stores the allowed characteristic handles for each connection. It clears them on `Disconnect`, on `ConnectionClosed` and on `CloseOwner`.

### 7. Mobile wiring

- `build.rs` runs `tauri_plugin::Builder::new(COMMANDS).android_path("android").ios_path("ios").build()`. The build generates the permission files.
- On Android, `init()` calls `register_android_plugin("dev.gattify.plugin", "GattifyPlugin")`. On iOS, `init()` calls `register_ios_plugin(init_plugin_gattify)`. Then `init()` sends the event channel.
- Android and iOS builds use the mobile backend. Other builds use the `Unsupported` backend.

## Lab app

`examples/gattify-lab` is a Tauri 2 app in plain TypeScript, with no UI framework. It is not published. It has five parts:

1. Adapter state, capabilities and permission requests.
2. Host: set a local name, then listen.
3. Join: a scan list. Tap a device to dial it.
4. Chat: send text to a peer. Show the time from send to ACK for each message.
5. A log of events and errors.

The app depends on the plugin crate and the npm package through paths in this repository. Its capability file grants `gattify:scan`, `gattify:peer` and `gattify:scope` with one lab service UUID.

## CI

The two existing jobs stay. Add two jobs:

- **android**: Ubuntu, JDK 17, the Android SDK and NDK. Build the lab app for Android in debug mode. This compiles the Kotlin code and the Rust code for Android.
- **ios**: macOS. Build the lab app for the iOS simulator, without code signing. This compiles the Swift code and the Rust code for iOS.

The iOS job runs on pull requests and on manual dispatch only. On a private repository, GitHub counts each macOS minute as ten minutes. The implementation plan for sub-project 1 confirms the exact build commands.

## Sub-projects

Each sub-project has its own implementation plan and its own pull request.

### 1. Foundation

- The workspace has one crate, `tauri-plugin-gattify`. Every name in the rename map is changed.
- ADR-002 records the crate layout and the platform decision.
- `build.rs` and the plugin registration exist.
- The native commands `execute` and `setEventChannel` exist. `execute` returns `Unsupported` for every radio command.
- The iOS managers start on first use.
- The lab app shows adapter state and capabilities.
- CI compiles the lab app for Android and for iOS.
- `cargo test`, `cargo clippy` and `npm test` pass.

### 2. Event path and peer driver

- `EventSink`, the event envelope, the event router and the webview cleanup exist.
- `MockAir` exists. Peer tests cover these cases:
  - the handshake
  - a 16 KiB message in each direction
  - a retry after a dropped frame
  - duplicate suppression
  - a close from each side
  - a lost link
  - a dial timeout
  - a second HELLO from one central
- The scope exists, with a test for each row of the scope table.
- The TypeScript package has `listen`, `onClose` and the new event names, with tests.
- `docs/protocol.md` defines the Info value, HELLO, HELLO_ACK and CLOSE.

### 3. Android backend

- The Kotlin code implements every `Command` and emits every `Event`, as the native bridge section states.
- Flow control, the MTU request and runtime permissions work as the native bridge rules state.
- CI compiles it.

### 4. iOS backend

- The Swift code implements every `Command` and emits every `Event`, as the native bridge section states.
- Flow control and lazy managers work as the native bridge rules state.
- CI compiles it.

Sub-projects 3 and 4 can run at the same time, because sub-project 2 fixes the contract.

### 5. Release preparation

- The README has a consumer section: the Cargo dependency, the npm dependency, the capability JSON, the iOS `NSBluetoothAlwaysUsageDescription` key and the Android permissions that the plugin manifest merges.
- The README shows the link wrapper from the TypeScript API section.
- The support matrix shows only verified evidence.
- CI runs `cargo deny check licenses`.
- `cargo publish --dry-run` and `npm pack --dry-run` pass.
- `CHANGELOG.md` has an entry for version `0.1.0-alpha.1`.

## Owner steps

1. Make the repository public before the first release. A public repository also makes the macOS CI minutes free.
2. After sub-projects 3 and 4, run the lab app on the iPhone and on the Android phone. Test each phone as host and as joiner. Record the results in `docs/platforms/test-results/`.
3. After sub-project 5, publish `0.1.0-alpha.1`.
4. After the phone test passes, publish `0.1.0`.

## Verification

| What | Where |
| --- | --- |
| Rust core, peer driver and scope | `cargo test` on this Mac and in CI |
| TypeScript facade | `npm test` on this Mac and in CI |
| Kotlin and Swift compilation | CI only. This Mac has no Android SDK and no full Xcode |
| Radio behavior | The owner's iPhone and Android phone |

## Risks

- A backgrounded iPhone host drops its local name and hides its service UUIDs from Android scanners. The foreground-only contract covers this case. An app must keep the host in the foreground.
- An Android advertisement holds 31 bytes. A 128-bit service UUID leaves about 8 bytes for the local name. The native layer puts the name in the scan response and cuts it at a UTF-8 boundary. `AdvertisingReport` reports the cut.
- The workspace lint forbids unsafe code. The `tauri::ios_plugin_binding!` macro can need a local exception. Sub-project 1 checks this.
- A write with a response costs one round trip for each frame. The message latency depends on the negotiated MTU. The lab app shows the time from send to ACK, so the owner can measure the latency before an app depends on it.
