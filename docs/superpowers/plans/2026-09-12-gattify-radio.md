# gattify radio implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Two phones exchange complete messages over BLE through gattify: sub-projects 2, 3 and 4 of `docs/superpowers/specs/2026-09-10-gattify-design.md`, plus the lab app and a physical test.

**Architecture:** Rust owns the peer protocol, the scope and event routing. Native code does raw GATT only, through the contract in `docs/native-bridge.md`. The peer driver is plain async Rust on tokio, so `MockAir` tests cover every peer flow with virtual time. The native backends depend only on the bridge contract, so they are built in parallel with the Rust work.

**Tech Stack:** Rust 1.89, Tauri 2.11, tokio 1.53 (already in the lock file through Tauri), TypeScript, Swift and CoreBluetooth, Kotlin and the Android BLE API.

---

## Decisions made while planning

These refine the approved spec. Each one closes a gap that the spec leaves open.

| Topic | Decision | Reason |
| --- | --- | --- |
| Driver runtime | The driver uses tokio directly: `tokio::spawn`, `tokio::time`, `tokio::sync`. tokio is a normal dependency with the `rt`, `sync` and `time` features | Tauri runs tokio already. Tests use `#[tokio::test(start_paused = true)]`, so the 5 s ACK and 10 s dial timeouts run in virtual time |
| Driver inbox | Events reach the driver through an unbounded mpsc channel. One driver task reads it | Native threads have no tokio context. `UnboundedSender::send` needs none |
| Per-peer writer | Each peer has one writer task with an ordered queue of outbound frames. The pump and the event handler both enqueue | HELLO_ACK, ACK and DATA leave in the order the driver decided. The joiner never sees DATA before HELLO_ACK |
| ACK deadline | The ACK deadline counts from the moment the last fragment is written. `Sender::mark_submitted` records it | A message with many fragments takes longer than 5 s to write at a small MTU. Without this change it retransmits forever |
| Connect from the driver | A device ID is connectable by any owner of the same owner family (`webview:main` and `gattify-peer:main`) | The webview scans, the driver dials. The spec rule "the same owner" would reject every dial |
| Characteristic key | `<serviceInstanceKey>/<characteristicInstanceKey>` | Instance keys are unique only inside a service |
| UUID form | Rust normalizes every UUID to lowercase 128-bit with hyphens before a command reaches native code | Native code compares strings only |
| Event payload | `gattify://<kind>` carries the event payload. `gattify://scan-result` carries `{ device }`, and the TypeScript scan handle reads `device` | One rule for every event |
| Webview isolation | Rust emits with `emit_to(EventTarget::webview(label))`. A listener with the default `Any` target in another webview of the same app still receives it | Tauri 2.11 delivers `Any` listeners every event. The first release targets single-webview mobile apps. `docs/native-bridge.md` and the README state the limit |
| Cleanup race | A page load starts owner cleanup and holds a per-label gate. Commands from that label wait for the gate | A reloaded page cannot lose its first scan to the cleanup of the old page |

## File map

| File | Responsibility |
| --- | --- |
| `crates/tauri-plugin-gattify/src/backend.rs` | `EventSink`, `Event::SubscriptionChanged.max_value_length` |
| `crates/tauri-plugin-gattify/src/uuid.rs` | `normalize_uuid`: 16, 32 and 128-bit forms to the canonical form |
| `crates/tauri-plugin-gattify/src/scope.rs` | `ServiceScope` and `ScopeGuard`: the scope table and the allowed characteristic handles |
| `crates/tauri-plugin-gattify/src/mock.rs` | `MockBackend` with an event sink, and `MockAir` |
| `crates/tauri-plugin-gattify/src/peer/sender.rs` | `in_flight`, `has_work`, `mark_submitted` |
| `crates/tauri-plugin-gattify/src/peer/driver.rs` | `PeerDriver`: endpoints, host flow, joiner flow, pump, writer, close |
| `crates/tauri-plugin-gattify/src/peer/driver_tests.rs` | The eight `MockAir` peer scenarios |
| `crates/tauri-plugin-gattify/src/events.rs` | Envelope parsing, routing, event names |
| `crates/tauri-plugin-gattify/src/commands.rs` | Scope checks, peer commands, setup, cleanup hooks |
| `crates/tauri-plugin-gattify/src/mobile.rs` | `setEventChannel` |
| `crates/tauri-plugin-gattify/permissions/scope.toml` | The `gattify:scope` permission |
| `crates/tauri-plugin-gattify/ios/Sources/*.swift` | iOS backend |
| `crates/tauri-plugin-gattify/android/src/main/java/dev/gattify/plugin/*.kt` | Android backend |
| `packages/plugin-gattify/src/{index,peer}.ts` | `listen`, `onClose`, the scan payload |
| `examples/gattify-lab/**` | The lab app: status, host, join, chat, log |
| `docs/protocol.md`, `docs/native-bridge.md` | Info value, HELLO, HELLO_ACK, CLOSE, native contract |

## Sub-project 2: event path, scope and peer driver (Rust)

### Task 2.1: tokio and the event sink

- [ ] Add `tokio = { version = "=1.53.1", default-features = false, features = ["rt", "sync", "time"] }` to `[workspace.dependencies]` and the crate. Add a dev-dependency with `macros`, `rt` and `test-util`.
- [ ] Add `pub type EventSink = Arc<dyn Fn(OwnerId, Event) + Send + Sync>;` to `backend.rs` and export it.
- [ ] Add `max_value_length: Option<u32>` to `Event::SubscriptionChanged`. Test that it serializes as `maxValueLength`.
- [ ] Update `fuzz/Cargo.lock` with `cargo update -p tokio --manifest-path fuzz/Cargo.toml`, then `cargo check --locked --manifest-path fuzz/Cargo.toml`.
- [ ] `cargo test --workspace --all-features`, commit.

### Task 2.2: UUID normalization

Tests first, in `uuid.rs`:

```rust
assert_eq!(normalize_uuid("180D").as_deref(), Some("0000180d-0000-1000-8000-00805f9b34fb"));
assert_eq!(normalize_uuid("0000180d").as_deref(), Some("0000180d-0000-1000-8000-00805f9b34fb"));
assert_eq!(normalize_uuid("80FF87C38E844914AEDC0D6A3BA5534D").as_deref(), Some("80ff87c3-8e84-4914-aedc-0d6a3ba5534d"));
assert_eq!(normalize_uuid("80ff87c3-8e84-4914-aedc-0d6a3ba5534d").as_deref(), Some("80ff87c3-8e84-4914-aedc-0d6a3ba5534d"));
assert_eq!(normalize_uuid("xyz"), None);
assert_eq!(normalize_uuid("180"), None);
```

- [ ] Implement `pub fn normalize_uuid(value: &str) -> Option<String>`: strip hyphens, require 4, 8 or 32 hex digits, expand 4 and 8 with the base UUID, lowercase, insert hyphens at 8-4-4-4-12.
- [ ] Commit.

### Task 2.3: scope

`ServiceScope::new(allow, deny)` keeps the normalized allowed UUIDs minus the denied ones. `ScopeGuard` stores `HashMap<(OwnerId, ConnectionId), HashSet<CharacteristicHandle>>`.

`ScopeGuard::authorize(&self, scope, owner, command) -> BleResult<Command>` returns the command with normalized UUIDs:

| Command | Rule |
| --- | --- |
| `StopScan`, `Disconnect`, `Unsubscribe`, `CloseServer`, `StopAdvertising` | Pass. Cleanup never reads the scope |
| any other command with an empty scope | `permissionDenied` |
| `StartScan` | The filter is not empty, and every UUID is in the scope |
| `Connect`, `DiscoverServices`, `SetValue`, `Notify` | Pass |
| `Read`, `Write`, `Subscribe` | The handle is in the set for `(owner, connection)` |
| `CreateServer` | Every service UUID is in the scope. Characteristic UUIDs are normalized, not checked |
| `StartAdvertising` | The service UUID is in the scope |

`ScopeGuard::observe(&self, scope, owner, command, reply) -> BleResult<Reply>` removes every service outside the scope from a `Services` reply and stores the remaining characteristic handles. It forgets the handles of a connection after `Disconnect`. `forget_connection` and `forget_owner` serve `ConnectionClosed` and `CloseOwner`.

- [ ] One test per row, plus: a handle from a filtered-out service is rejected, `Disconnect` clears handles, `forget_owner` clears handles.
- [ ] Implement, commit.

### Task 2.4: MockAir

`MockBackend::new(sink)` and `MockBackend::default()` (no-op sink) keep the existing behavior for tests without a link. `MockAir::link(sink_a, sink_b) -> (MockAir, MockBackend, MockBackend)` joins two sides:

- Each side sees the other as device `mock-device-<side>` and appears to the other's server as central `mock-central-<side>`.
- `StartScan` emits one `ScanResult` for the other side when it advertises a matching service.
- `Connect` to the other device links to its advertising server. `Connected` reports the configured value limit, default 20.
- `DiscoverServices` returns the server definition with handles. `Read` returns the stored value.
- `Write` on the other side's characteristic emits `ServerWrite` there. `Subscribe` and `Unsubscribe` emit `SubscriptionChanged` there, with `max_value_length`. `Disconnect` emits `SubscriptionChanged { subscribed: false }` for each subscription.
- `Notify` to a subscribed central emits `CharacteristicValue` on its side.
- `MockAir::drop_next_frame(side)` drops the next `Write` or `Notify` sent by that side: the command succeeds, and nothing arrives.
- `MockAir::disconnect()` closes every link: `ConnectionClosed` to the central, `SubscriptionChanged { subscribed: false }` to the server.
- `MockAir::set_value_limit(n)` changes the reported limits.
- Events are emitted after the state lock is released.

- [ ] Tests: a write arrives as `ServerWrite`, a notify arrives as `CharacteristicValue`, a dropped frame does not arrive, `disconnect` emits both events, the existing owner-cleanup tests still pass.
- [ ] Implement, commit.

### Task 2.5: Sender refinements

- [ ] Tests: `in_flight()` is the pending ID; `has_work()` is false only with no pending and no queued message; after `mark_submitted(id, 4_000)`, `poll(8_999)` is `Idle` and `poll(9_000)` retransmits.
- [ ] Implement, commit.

### Task 2.6: peer driver

Public surface in `peer/driver.rs`:

```rust
pub struct EndpointOptions { pub service_uuid: String, pub local_name: Option<String>, pub max_logical_payload: usize, pub listen: bool }
pub enum CloseReason { Local, Remote, Lost }
pub enum PeerEvent {
    Ready { endpoint_id: String, peer_id: PeerId },
    Message { peer_id: PeerId, bytes: Vec<u8> },
    Closed { peer_id: PeerId, reason: CloseReason },
}
impl PeerEvent { pub fn name(&self) -> &'static str; pub fn payload(&self) -> serde_json::Value; }
pub type PeerEmitter = Arc<dyn Fn(&str, PeerEvent) + Send + Sync>;
pub struct SendReceipt { pub message_id: u32, pub delivery: DeliveryOutcome }

impl PeerDriver {
    pub fn new(runtime: BleRuntime, emit: PeerEmitter) -> Self;
    pub async fn run(self, inbox: UnboundedReceiver<(OwnerId, Event)>);
    pub async fn create_endpoint(&self, label: &str, options: EndpointOptions) -> BleResult<String>;
    pub async fn dial(&self, label: &str, endpoint_id: &str, device_id: DeviceId) -> BleResult<PeerId>;
    pub async fn send(&self, label: &str, peer_id: &PeerId, bytes: Vec<u8>, timeout: Duration) -> BleResult<SendReceipt>;
    pub async fn close_peer(&self, label: &str, peer_id: &PeerId) -> BleResult<()>;
    pub async fn close_endpoint(&self, label: &str, endpoint_id: &str) -> BleResult<()>;
    pub fn forget_label(&self, label: &str);
}
```

The host server has one service with the instance key `peer`: `info` (read, initial `0x01`, max 1), `rx` (write, max 512) and `tx` (notify, max 512). The owner is `gattify-peer:<label>`.

Scenario tests in `peer/driver_tests.rs`, each with `#[tokio::test(start_paused = true)]` over `MockAir`:

1. Handshake: the host listens, the joiner dials, the dial returns a peer, the host emits `Ready`.
2. A 16 KiB message in each direction arrives intact, and each send returns `transportAcknowledged`.
3. A dropped DATA frame is retransmitted after 5 s, and the message arrives once.
4. A dropped ACK makes the joiner retransmit. The host ACKs the duplicate and emits the message once.
5. `close_peer` on the joiner: the joiner emits `Closed(local)`, the host emits `Closed(remote)`. The same test from the host side: the joiner emits `Closed(remote)` and disconnects.
6. `MockAir::disconnect`: both sides emit `Closed(lost)`, and a pending send fails with `disconnected`.
7. A dropped HELLO_ACK makes `dial` fail with `timeout` after 10 s, and the joiner disconnects.
8. A second HELLO from one central closes the first peer with `lost` and emits a new `Ready`.

Also: a peer ID from another label is `invalidHandle`; `close_endpoint` leaves the host owner with no server.

- [ ] Write the tests, watch them fail, implement, watch them pass, commit.

### Task 2.7: event router and plugin wiring

- [ ] `events.rs`: `Envelope { owner_id, event }`, `route(&OwnerId) -> Route { Peer(label) | Webview(label) | Drop }`, `event_name(&Event) -> String` (`gattify://scan-result`). Unit tests for both.
- [ ] `commands.rs`: `GattifyState { runtime, driver, guard, gates }`; role check, then cleanup gate, then scope; the five peer commands; `on_page_load` (Started) and `RunEvent::WindowEvent { Destroyed }` cleanup.
- [ ] `mobile.rs`: create the event channel and send `setEventChannel` from setup on a spawned task.
- [ ] `permissions/scope.toml`: `[[permission]] identifier = "scope"` with no commands.
- [ ] Lab IPC tests: a peer command with no scope rejects with `permissionDenied`; a scan with an out-of-scope UUID rejects.
- [ ] Commit.

### Task 2.8: TypeScript

- [ ] `PeerOptions.listen` (sent as `listen`, default false), `Peer.onClose`, the scan handle reads `payload.device`. Tests in `packages/plugin-gattify/test/facade.test.mjs`.
- [ ] Commit.

### Task 2.9: protocol document

- [ ] `docs/protocol.md`: the Info value `0x01`, HELLO and HELLO_ACK without payload, CLOSE without reply, the ACK deadline from the last fragment, the joiner and host flows.
- [ ] Commit.

## Sub-project 3: Android backend

- [ ] Implement every command and event of `docs/native-bridge.md` in Kotlin under `crates/tauri-plugin-gattify/android/src/main/java/dev/gattify/plugin/`.
- [ ] Keep the JSON encoding and pure helpers testable in JUnit: UUID expansion, name cutting, ID allocation, reply builders.
- [ ] Verify: `npm run tauri --workspace examples/gattify-lab -- android build --debug --apk --target aarch64`, then `./gradlew :tauri-plugin-gattify:testDebugUnitTest` in `examples/gattify-lab/src-tauri/gen/android`.

## Sub-project 4: iOS backend

- [ ] Implement every command and event of `docs/native-bridge.md` in Swift under `crates/tauri-plugin-gattify/ios/Sources/`.
- [ ] Keep pure helpers testable in XCTest.
- [ ] Verify: `cargo build -p tauri-plugin-gattify --target aarch64-apple-ios`, then `xcodebuild test -scheme tauri-plugin-gattify -destination id=<simulator> -skipPackagePluginValidation` in `crates/tauri-plugin-gattify/ios`.

## Lab app

- [ ] Capability: `gattify:default`, `gattify:scan`, `gattify:peer`, `gattify:allow-request-connect-permission`, `gattify:allow-request-advertise-permission`, and `gattify:scope` with the lab service UUID `80ff87c3-8e84-4914-aedc-0d6a3ba5534d`.
- [ ] iOS `NSBluetoothAlwaysUsageDescription`.
- [ ] Five parts: status and permissions, host, join, chat with send-to-ACK time, event log.

## Physical test

- [ ] Build the lab app for both iPhones and install it over the local network.
- [ ] Each phone as host and as joiner: handshake, a short message each way, a 4 KiB message each way, close from each side.
- [ ] Record the results in `docs/platforms/test-results/`.
