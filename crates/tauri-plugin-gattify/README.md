# tauri-plugin-gattify

The Rust plugin of [gattify](https://github.com/hoangmirs/gattify), a Tauri v2
plugin for Bluetooth Low Energy. It runs raw GATT operations and sends
complete messages between two devices on Android, iOS, macOS and Windows.
Linux returns `Unsupported` for every radio command.

The frontend calls the plugin through the npm package
[`tauri-plugin-gattify-api`](https://www.npmjs.com/package/tauri-plugin-gattify-api).

## Install

In the app's `src-tauri` folder:

```sh
cargo add tauri-plugin-gattify
```

Register the plugin:

```rust
tauri::Builder::default()
    .plugin(tauri_plugin_gattify::init())
    .run(tauri::generate_context!())
    .expect("failed to run application");
```

## Permissions

`gattify:default` allows only status queries, the event channel and owner
cleanup. Grant each role the app uses: `gattify:scan`, `gattify:connect`,
`gattify:server`, `gattify:advertise` and `gattify:peer`.

`gattify:scope` lists the service UUIDs the app allows. An empty scope rejects
every radio command.

```json
{
  "identifier": "gattify:scope",
  "allow": [{ "serviceUuid": "80ff87c3-8e84-4914-aedc-0d6a3ba5534d" }]
}
```

## Features

- `tauri` (default): the Tauri commands and the native backends.
- `mock`: a deterministic mock backend for tests. `init` never selects it.

The [gattify README](https://github.com/hoangmirs/gattify#readme) explains the
platform setup, the platform status and the security limits.

## License

MIT
