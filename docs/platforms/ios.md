# iOS

Target baseline: iOS 15. The plugin owns central and peripheral managers on a
dedicated queue and maps CoreBluetooth state without inferring capabilities.

Current native source provides state and honest unknown capability probes.
Scanning, connections, service discovery, cached reads, bounded writes,
subscription tracking, ready-to-update flow control and targeted notification
qualification remain unimplemented.

