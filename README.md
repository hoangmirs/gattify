# gattify

[![CI](https://github.com/hoangmirs/gattify/actions/workflows/ci.yml/badge.svg?branch=develop)](https://github.com/hoangmirs/gattify/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/tauri-plugin-gattify.svg)](https://crates.io/crates/tauri-plugin-gattify)
[![npm](https://img.shields.io/npm/v/tauri-plugin-gattify-api.svg)](https://www.npmjs.com/package/tauri-plugin-gattify-api)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A Tauri v2 plugin for Bluetooth Low Energy. It gives an app raw GATT
operations: scan, connect, read, write, subscribe, a local GATT server and
advertising. An optional peer transport sends complete messages between two
devices.

Version 0.1.0 is on crates.io and npm. The Android, iOS, macOS and Windows
backends implement every command of the native contract. Most radio behavior
has not run on hardware yet. Linux returns `Unsupported` for every radio
command.

| Platform | Backend | Minimum | Hardware verified |
| --- | --- | --- | --- |
| Android | Kotlin | API 26 | No |
| iOS | Swift on CoreBluetooth | iOS 15 | Partly: peripheral role and peer host |
| macOS | The iOS Swift engine through a C ABI | The app's deployment target, else 10.15 | No |
| Windows | Rust on WinRT | Windows 10 1903 | No |
| Linux | None | — | — |

`docs/support-matrix.md` and `IMPLEMENTATION_STATUS.md` record the evidence
for each claim.

## Packages

| Package | Registry | Contents |
| --- | --- | --- |
| `tauri-plugin-gattify` | crates.io | The Rust plugin: commands, permissions, native backends, mock backend, peer protocol and driver |
| `tauri-plugin-gattify-api` | npm | The TypeScript API. Raw GATT at the package root, peers at `/peer` |

## Install

In the app's `src-tauri` folder:

```sh
cargo add tauri-plugin-gattify
```

In the app's frontend folder:

```sh
npm install tauri-plugin-gattify-api
```

The npm package has a peer dependency on `@tauri-apps/api` 2.11.1.

Register the plugin:

```rust
tauri::Builder::default()
    .plugin(tauri_plugin_gattify::init())
    .run(tauri::generate_context!())
    .expect("failed to run application");
```

## Grant permissions

Each role has its own permission set. Grant only the roles the app uses.

| Permission | Allows |
| --- | --- |
| `gattify:default` | Status queries, the event channel and owner cleanup |
| `gattify:scan` | Scans and the scan permission prompt |
| `gattify:connect` | Connections, GATT client operations and the connect permission prompt |
| `gattify:server` | A local GATT server |
| `gattify:advertise` | Advertising and the advertise permission prompt |
| `gattify:peer` | Peer endpoints: listen, dial, send and close |
| `gattify:scope` | The service UUIDs the app allows |

The Rust side validates each role command again after Tauri authorizes it.

`gattify:scope` limits every radio command to the listed service UUIDs:

- An empty scope rejects every radio command.
- A scan needs a service filter inside the scope.
- A discovery returns only services inside the scope.
- A server or an advertisement can use only UUIDs inside the scope.

This capability is the one the lab app uses to scan, host and dial peers:

```json
{
  "identifier": "default",
  "windows": ["main"],
  "permissions": [
    "core:default",
    "gattify:default",
    "gattify:scan",
    "gattify:peer",
    "gattify:allow-request-connect-permission",
    "gattify:allow-request-advertise-permission",
    {
      "identifier": "gattify:scope",
      "allow": [{ "serviceUuid": "80ff87c3-8e84-4914-aedc-0d6a3ba5534d" }]
    }
  ]
}
```

## Platform setup

- **iOS**: add `NSBluetoothAlwaysUsageDescription` to
  `src-tauri/Info.ios.plist`.
- **macOS**: add `NSBluetoothAlwaysUsageDescription` to
  `src-tauri/Info.plist`. A sandboxed app also needs the
  `com.apple.security.device.bluetooth` entitlement.
- **Android**: the plugin manifest merges the Bluetooth permissions. A peer
  host needs both the connect and the advertise permission.
- **Windows**: a packaged app declares the `bluetooth` capability in its
  manifest. An unpackaged app needs no setup.

`docs/platforms/` gives the requirements and limits of each platform.

## Use raw GATT

A frontend opens a session and closes it explicitly. Importing the package
does not start Bluetooth.

```ts
import { createBle } from "tauri-plugin-gattify-api";

const ble = await createBle();
await ble.requestPermissions({ scan: true, connect: true, advertise: false });

const scan = await ble.scan({ serviceUuids: [SERVICE_UUID] });
scan.onUpdate((device) => console.log(device.name, device.id));
await scan.stop();

const connection = await ble.connect(device.id);
const services = await connection.discoverServices();
const value = await connection.read(characteristic);
await connection.close();

await ble.close();
```

A session also runs `write`, `subscribe`, `createServer`, `startAdvertising`
and `notify`. `packages/plugin-gattify/src/index.ts` declares the full API.

## Send messages between two devices

A peer endpoint sends and receives complete messages. The host listens. The
joiner dials a device from its scan.

```ts
import { createEndpoint } from "tauri-plugin-gattify-api/peer";

const host = await createEndpoint({ serviceUuid: SERVICE_UUID, localName: "Host", listen: true });
host.onPeer((peer) => {
  peer.onMessage((bytes) => console.log(new TextDecoder().decode(bytes)));
  peer.onClose((reason) => console.log("closed", reason));
});

const joiner = await createEndpoint({ serviceUuid: SERVICE_UUID });
const peer = await joiner.dial(device.id);
const { delivery } = await peer.send(new TextEncoder().encode("hello"));
```

`send` resolves with `delivery: "transportAcknowledged"` when the other device
accepted the complete message. A failed send rejects with a `BleError`. Its
`delivery` is `notSubmitted` or `unknown`.

## Behavior

- Each webview receives its events through its own Tauri channel. Another
  webview of the app cannot observe them.
- A page reload releases every scan, connection, server and peer of the old
  page.
- `scan` and `connect` accept an `AbortSignal`. The signal sends a
  cancellation to the native operation. A backend may be unable to stop an OS
  procedure immediately. That backend must still ignore and clean up a late
  completion.
- A device ID is an opaque runtime handle. It is not a MAC address or a
  durable identity.

## Security and limits

The peer profile is unencrypted and unauthenticated. Use it only for public
test data until a separately reviewed secure-session layer exists.

A transport acknowledgement means the next device accepted the complete
message. It does not mean that the application processed or stored the
message, or that the final recipient received it.

Version 0.1 runs in the foreground only. Bluetooth Classic, exact ranging,
background delivery, L2CAP, mesh and internet fallback are outside v0.1.

`SECURITY.md` gives the security policy.

## Examples

- `examples/gattify-lab`: the two-device test harness. It shows adapter
  state and permissions, hosts, scans and joins, and chats with the time from
  send to ACK. Its README gives the build steps.
- `examples/offline-chat`: an unencrypted group chat model at the application
  layer. One coordinator orders the messages and tracks the receipts.
- `examples/nearby-invite`: an invitation inbox that records each decision
  once.

## Development

Requirements: Node 22+, npm 11+, Rust 1.89, and the platform SDKs for native
builds.

```sh
npm install
npm test
npm run build
cargo fmt --all --check
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --manifest-path examples/gattify-lab/src-tauri/Cargo.toml
```

On a Mac, `crates/tauri-plugin-gattify/macos/test.sh` runs the Swift engine
tests.

`CONTRIBUTING.md` explains how to contribute.

## Documentation

- `docs/native-bridge.md`: the contract between Rust and the native backends
- `docs/protocol.md`: the peer wire protocol
- `docs/platforms/`: the requirements and limits of each platform
- `docs/adr/`: the architecture decisions
- `CHANGELOG.md`: the changes in each release

## License

MIT
