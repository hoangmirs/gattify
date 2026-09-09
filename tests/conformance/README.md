# Backend conformance

Every production backend must run the same owner/resource lifecycle suite:

1. status and capability probes never fabricate support;
2. start/stop and close operations are idempotent;
3. stale and cross-owner handles are rejected;
4. cancellation resolves once and late successful connects are cleaned up;
5. adapter reset fails pending operations and invalidates generations;
6. targeted notification addresses exactly one subscribed peer;
7. a stalled peer cannot block another connection indefinitely;
8. critical event overflow fails visibly instead of dropping state.

The deterministic mock exercises the portable lifecycle contract in ble-core.
Hardware runners must write dated evidence under docs/platforms/test-results.

