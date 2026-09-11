# Android

Target baseline: API 26+, compile SDK 36, Java 17. Android 12+ requires
BLUETOOTH_SCAN, BLUETOOTH_CONNECT and BLUETOOTH_ADVERTISE by requested role.
Older versions may require location permission for scanning.

Every command reaches the native `execute` command. The native source reads
adapter state and reports capabilities as unknown. Scan, connection, GATT
client/server callbacks, cached ATT reads, write validation, notification
completion, and per-subscriber targeting remain unimplemented and return
Unsupported through the Rust backend.

