# TypeScript facade

This private package is the framework-neutral frontend API for the local
tauri-plugin-ble workspace. The final npm scope and publisher must be selected
before publication.

Import raw GATT operations from the package root and complete-message peer
operations from tauri-plugin-ble-api/peer. Importing either module performs no
Bluetooth work.
