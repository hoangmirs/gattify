# Offline chat example

This application-layer example is intentionally unencrypted and suitable only
for public test messages. It demonstrates admission, coordinator-owned sequence
numbers, per-recipient receipts, bounded history, gap reporting, and
connection-scoped deduplication.

The coordinator is a logical role. Every participant needs a direct BLE peer
link to it. Coordinator departure stops group delivery; there is no automatic
host migration. Transport acceptance, coordinator acceptance, recipient
delivery, and read state remain distinct outcomes.

