# gattify lab

This example is the manual conformance harness for two devices: two phones, or
a phone and a Mac or Windows PC.

1. **Adapter**: state, permissions and role support. Request the scan,
   connect and advertise permissions here first.
2. **Host**: set a local name and listen. The device advertises the lab service.
3. **Join**: scan for the lab service and tap a host to dial it.
4. **Chat**: send text or 4 KiB to the selected peer. Each sent message shows
   the time from send to transport ACK.
5. **Log**: handles, outcomes and errors, never payload contents.

The capability file grants `gattify:scan`, `gattify:peer`, the connect and
advertise permission prompts, and `gattify:scope` with the lab service UUID
`80ff87c3-8e84-4914-aedc-0d6a3ba5534d`. `Info.ios.plist` and `Info.plist` add
the Bluetooth usage description on iOS and macOS.

Do not use the mock to record hardware evidence.

Run it on a Mac or a Windows PC from the repository root:

    npm run build
    npm run tauri --workspace examples/gattify-lab -- dev

On macOS the first scan or host shows the Bluetooth prompt. A Mac does not see
its own advertisement, so pair it with another device. On Linux every radio
command reports Unsupported.

Build it for a phone with `tauri ios build` or `tauri android build`. An iOS
build needs a development team: set `APPLE_DEVELOPMENT_TEAM`.
