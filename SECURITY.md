# Security policy

Report vulnerabilities privately to the maintainer address selected before
publication. Do not include BLE payloads, device identifiers, keys, or personal
data in public reports.

v0.1 peer transport is explicitly unauthenticated and unencrypted. Do not use it
for private chat, credentials, personal data, or secrets. BLE link protection is
reported only when observable and does not replace application authentication.

The implementation validates UUIDs, base64, frame metadata and allocation
bounds. Logs must never include user payloads. Diagnostics are local and opt-in.

