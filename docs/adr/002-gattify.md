# ADR-002: gattify crate layout and mobile backends

Status: accepted
Date: 2026-09-10

## Decision

Publish one Rust crate, `tauri-plugin-gattify`. The files of the former `ble-core` crate move to the crate root. The former `ble-peer` crate becomes the `peer` module. The crate root holds the core files because a module named `core` makes `core::` paths ambiguous with the Rust `core` crate.

The modules `backend`, `error`, `manager`, `model` and `peer` contain no Tauri, OS SDK or WebView types. The `tauri` feature is on by default and adds the Tauri commands and the mobile backend. Unit tests and the fuzz target build without that feature.

Android and iOS backends use the Tauri mobile plugin API: Kotlin on Android and Swift on iOS. The native code implements raw GATT commands only. The peer protocol runs in Rust. Desktop builds keep a backend that returns Unsupported for radio operations.

The Android library compiles against SDK 36, as the Tauri Android library does. The minimum stays at API 26. The iOS minimum stays at iOS 15.

## Consequences

One crate is one package to publish and version. Apps depend on `tauri-plugin-gattify` and on the npm package `tauri-plugin-gattify-api`. A desktop radio backend needs a new decision record.

This record replaces the crate layout and the desktop backend plan of ADR-001.
