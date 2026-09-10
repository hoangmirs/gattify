# gattify lab

This example is the manual conformance harness. It shows adapter state and
capabilities today. Later sub-projects add permissions, scan, host and join,
a chat that sends text to a peer, and the time from send to ACK.

Do not use the mock to record hardware evidence. Each operation should show its
opaque handle, operation deadline, native outcome, and actual link value limits
without logging payload contents.

Run it on the desktop from the repository root:

    npm run build
    npm run tauri --workspace examples/gattify-lab -- dev
