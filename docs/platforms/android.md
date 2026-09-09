# Android

Target baseline: API 26+, compile SDK 35, Java 17. Android 12+ requires
BLUETOOTH_SCAN, BLUETOOTH_CONNECT and BLUETOOTH_ADVERTISE by requested role.
Older versions may require location permission for scanning.

Current native source probes adapter state, advertiser availability and basic
capabilities. Scan, connection, GATT client/server callbacks, cached ATT reads,
write validation, notification completion, and per-subscriber targeting remain
unimplemented and must return Unsupported through the Rust backend.

