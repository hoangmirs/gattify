# iOS

Target baseline: iOS 15. Every command reaches the native `execute` command.
The native source reads the Bluetooth authorization without a prompt,
reports capabilities as unknown, and creates no CoreBluetooth manager yet.

Scanning, connections, service discovery, cached reads, bounded writes,
subscription tracking, ready-to-update flow control and targeted notification
qualification remain unimplemented.

