# tauri-plugin-gattify-api

The TypeScript API of [gattify](https://github.com/hoangmirs/gattify), a
Tauri v2 plugin for Bluetooth Low Energy. It runs raw GATT operations and
sends complete messages between two devices on Android, iOS, macOS and
Windows. Linux returns `Unsupported` for every radio operation.

This package needs the Rust plugin
[`tauri-plugin-gattify`](https://crates.io/crates/tauri-plugin-gattify) in the
app, and the `gattify:*` permissions in its capabilities.

## Install

```sh
npm install tauri-plugin-gattify-api
```

The package has a peer dependency on `@tauri-apps/api` 2.11.1.

## Use

Raw GATT operations come from the package root. Importing the package does
not start Bluetooth.

```ts
import { createBle } from "tauri-plugin-gattify-api";

const ble = await createBle();
await ble.requestPermissions({ scan: true, connect: true, advertise: false });
const scan = await ble.scan({ serviceUuids: [SERVICE_UUID] });
scan.onUpdate((device) => console.log(device.name, device.id));
await scan.stop();
await ble.close();
```

Peer messages come from `tauri-plugin-gattify-api/peer`:

```ts
import { createEndpoint } from "tauri-plugin-gattify-api/peer";

const endpoint = await createEndpoint({ serviceUuid: SERVICE_UUID });
const peer = await endpoint.dial(device.id);
const { delivery } = await peer.send(new TextEncoder().encode("hello"));
```

The peer profile is unencrypted and unauthenticated. Use it only for public
test data.

The [gattify README](https://github.com/hoangmirs/gattify#readme) explains the
permissions, the platform setup and the platform status.

## License

MIT
