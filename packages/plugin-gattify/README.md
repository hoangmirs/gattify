# TypeScript facade

The framework-neutral frontend API for the gattify Tauri plugin. Version
0.1.0-alpha.2 runs raw GATT operations and complete-message peers on Android,
iOS, macOS and Windows. Linux builds return Unsupported for every radio
operation.

Import raw GATT operations from the package root and complete-message peer
operations from tauri-plugin-gattify-api/peer. Importing either module performs
no Bluetooth work.
