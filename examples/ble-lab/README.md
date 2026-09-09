# BLE Lab

This example is the manual conformance harness. It will expose adapter state,
capabilities, permissions, scan, connection, service discovery, read/write,
subscription, server registration, advertising, targeted echo, and sanitized
queue diagnostics.

The UI is not packaged yet because no native backend is implemented. Do not use
the mock to record hardware evidence. When M2 begins, each operation should show
its opaque handle, operation deadline, native outcome, and actual link value
limits without logging payload contents.

