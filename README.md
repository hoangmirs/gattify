# gattify

A Tauri v2 BLE plugin for raw GATT operations and optional
complete-message transport.

This repository is an implementation-in-progress. The platform-neutral
contracts, the deterministic mock backend, the TypeScript facade, the peer wire
protocol and the peer driver are implemented and tested. The Kotlin backend on
Android and the Swift backend on iOS implement every command and event of
`docs/native-bridge.md`. Desktop builds keep an explicit Unsupported backend.
IMPLEMENTATION_STATUS.md records which radio behavior has run on hardware.

## Packages

| Package | Status | Purpose |
| --- | --- | --- |
| tauri-plugin-gattify | Implemented; Android and iOS backends await full hardware qualification | Rust crate: DTOs, errors, ownership, scope, backend contract, mock, peer protocol and driver, Tauri commands |
| tauri-plugin-gattify-api | Implemented and tested | Framework-neutral TypeScript handles |

The npm package stays private until the first release.

## Development

Requirements: Node 22+, npm 11+, Rust 1.89, and platform SDKs for native builds.

    npm install
    npm test
    npm run build
    cargo fmt --all --check
    cargo test --workspace --all-features
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    cargo test --manifest-path examples/gattify-lab/src-tauri/Cargo.toml

A consumer registers the Rust plugin:

    tauri::Builder::default()
        .plugin(tauri_plugin_gattify::init())
        .run(tauri::generate_context!())
        .expect("failed to run application");

The frontend owns a session and closes it explicitly:

    import { createBle } from "tauri-plugin-gattify-api";

    const ble = await createBle();
    await ble.requestPermissions({ scan: true, connect: true, advertise: true });
    const scan = await ble.scan({ serviceUuids: [SERVICE_UUID] });
    scan.onUpdate((device) => console.log(device.name, device.id));
    // ...
    await scan.stop();
    await ble.close();

Two phones exchange complete messages through a peer endpoint. The host
listens; the joiner dials a device from its scan:

    import { createEndpoint } from "tauri-plugin-gattify-api/peer";

    const host = await createEndpoint({ serviceUuid: SERVICE_UUID, localName: "Host", listen: true });
    host.onPeer((peer) => {
      peer.onMessage((bytes) => console.log(new TextDecoder().decode(bytes)));
      peer.onClose((reason) => console.log("closed", reason));
    });

    const joiner = await createEndpoint({ serviceUuid: SERVICE_UUID });
    const peer = await joiner.dial(device.id);
    const { delivery } = await peer.send(new TextEncoder().encode("hello"));

`send` resolves with `transportAcknowledged` once the other side admitted the
complete message. Importing the package does not initialize Bluetooth.

Tauri capabilities opt into roles separately with `gattify:scan`,
`gattify:connect`, `gattify:server`, `gattify:advertise`, and `gattify:peer`.
The default `gattify:default` permission exposes status queries, the event
channel and owner cleanup only. The Rust boundary validates each role-specific
command even after Tauri authorizes it.

`gattify:scope` lists the service UUIDs that the app allows. An empty scope
rejects every radio command. A scan needs a filter inside the scope, a
discovery returns only services inside it, and a server or advertisement can
use only its UUIDs:

    {
      "identifier": "gattify:scope",
      "allow": [{ "serviceUuid": "80ff87c3-8e84-4914-aedc-0d6a3ba5534d" }]
    }

A webview receives its events through its own Tauri channel, so another
webview of the app cannot observe them. A page reload releases every scan,
connection, server and peer of the old page.

On iOS the app needs `NSBluetoothAlwaysUsageDescription`; with Tauri, put it in
`src-tauri/Info.ios.plist`. On Android the plugin manifest merges the Bluetooth
permissions; a host needs both the connect and the advertise permission.

`scan` and `connect` forward a caller's `AbortSignal` to an owner-scoped native
cancellation command using the same operation ID. A backend may be unable to
interrupt an OS procedure immediately; late completions must still be ignored
and cleaned up by that backend.

The lab app in examples/gattify-lab is the two-phone test harness: adapter
state and permissions, host, scan and join, chat with the time from send to
ACK, and a log. See its README for the build steps.

## Safety and scope

The peer profile is unencrypted and unauthenticated. It is suitable only for
public test data until a separately reviewed secure-session layer exists.
Transport acknowledgement means the next device accepted the complete message;
it does not mean application processing, persistence, or final-recipient
delivery.

Device identifiers are opaque runtime handles, not MAC addresses or durable
identities. The first contract is foreground-only. Bluetooth Classic, exact
ranging, automatic background delivery, L2CAP, mesh, and internet fallback are
outside v0.1.

See IMPLEMENTATION_STATUS.md for exact evidence and docs/support-matrix.md for
platform claims.
