# ADR-001: Foundation versions and backend boundary

Status: accepted for initial implementation. ADR-002 replaces its crate layout and its desktop backend plan.
Date: 2026-09-08

## Decision

Use a Cargo workspace with Rust 1.89 and edition 2021. Keep ble-core and
ble-peer free of Tauri and OS SDK types. Use Tauri 2.11.x at the plugin boundary,
Android API 26 minimum with compile SDK 35 and Java 17, iOS 15 minimum, and
Node 22 with TypeScript 5.9 for the frontend package.

Native APIs are the intended production backends: Kotlin Android Bluetooth
APIs, Swift CoreBluetooth on iOS, a distinct macOS CoreBluetooth integration,
Windows GATT APIs, and BlueZ D-Bus on Linux. Until each adapter is implemented
and qualified it reports unknown capabilities and returns Unsupported. The mock
backend is dependency-injected only in tests.

Use one externally tagged command/reply envelope between the Rust manager and
backends. Every operation carries an opaque owner and operation ID. Native
adapters must serialize procedures per connection while allowing independent
connections to progress concurrently.

## Consequences

The core and peer crates remain portable and testable without mobile SDKs.
Platform progress can be reported accurately. The command envelope may need a
future versioned compatibility layer before a stable 1.0 API.

