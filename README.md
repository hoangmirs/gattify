# gattify

A Tauri v2 BLE plugin for raw GATT operations and optional
complete-message transport.

This repository is an implementation-in-progress. The platform-neutral
contracts, deterministic mock backend, TypeScript facade, and peer wire
protocol are implemented. Android and iOS contain native state/capability
probes. No production radio backend is currently claimed as complete. Every
unimplemented production operation returns an explicit Unsupported error.

## Packages

| Package | Status | Purpose |
| --- | --- | --- |
| tauri-plugin-gattify | API surface implemented; native GATT backends gated | Rust crate: DTOs, errors, ownership, backend contract, mock, peer framing and Tauri commands |
| tauri-plugin-gattify-api | Implemented and tested | Framework-neutral TypeScript handles |

The npm package stays private until the first release.

## Development

Requirements: Node 22+, npm 11+, Rust 1.89, and platform SDKs for native builds.

    npm install
    npm test
    cargo fmt --all --check
    cargo test --workspace --all-features
    cargo clippy --workspace --all-targets --all-features -- -D warnings

A consumer registers the Rust plugin:

    tauri::Builder::default()
        .plugin(tauri_plugin_gattify::init())
        .run(tauri::generate_context!())
        .expect("failed to run application");

The frontend owns a session and closes it explicitly:

    import { createBle } from "tauri-plugin-gattify-api";

    const ble = await createBle();
    const capabilities = await ble.getCapabilities();
    const scan = await ble.scan({ serviceUuids: [] });
    // ...
    await scan.stop();
    await ble.close();

Importing the package does not initialize Bluetooth. Peer imports return
Unsupported until a peer-capable native backend exists.

Tauri capabilities opt into roles separately with `gattify:scan`,
`gattify:connect`, `gattify:server`, `gattify:advertise`, and `gattify:peer`.
The default `gattify:default` permission exposes status queries and owner
cleanup only. The Rust boundary validates each role-specific command even
after Tauri authorizes it.

`scan` and `connect` forward a caller's `AbortSignal` to an owner-scoped native
cancellation command using the same operation ID. A backend may be unable to
interrupt an OS procedure immediately; late completions must still be ignored
and cleaned up by that backend.

The lab app in examples/gattify-lab shows adapter state and capabilities.
Run it on the desktop with `npm run tauri --workspace examples/gattify-lab -- dev`
after `npm run build`.

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
