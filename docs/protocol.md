# BLE peer protocol v1

The profile provides reliable, bounded complete messages over GATT. It is not
authenticated or encrypted.

The application supplies the discovery service UUID. Fixed characteristic UUIDs:

| Characteristic | UUID | Properties |
| --- | --- | --- |
| Info | b1e10f10-6a2c-4a62-8e9e-2c938fa30101 | Read |
| RX | b1e10f10-6a2c-4a62-8e9e-2c938fa30102 | Write with response |
| TX | b1e10f10-6a2c-4a62-8e9e-2c938fa30103 | Notify |

All integers are little-endian. No in-memory struct serialization is permitted.

| Offset | Bytes | Field |
| --- | --- | --- |
| 0 | 1 | major = 1 |
| 1 | 1 | HELLO=1, HELLO_ACK=2, DATA=3, ACK=4, CLOSE=5 |
| 2 | 4 | message ID |
| 6 | 2 | fragment index |
| 8 | 2 | fragment count |
| 10 | 4 | total logical length |
| 14 | remaining | fragment bytes |

DATA IDs are nonzero. ACK uses the acknowledged ID, index 0, count 1, total
length 0, and no payload. Other control messages use ID 0. Empty DATA is one
header-only frame. Values below 14 bytes are unsupported. At a 20-byte value
limit each non-final fragment carries six bytes.

Golden DATA vector for message 0x12345678, fragment 1 of 3, total length 7,
payload aa bb:

    01 03 78 56 34 12 01 00 03 00 07 00 00 00 aa bb

Golden ACK vector:

    01 04 78 56 34 12 00 00 01 00 00 00 00 00

Defaults: 16 KiB logical message, 64 KiB outbound per peer, 1 MiB protocol
buffers per adapter, 5 s ACK deadline, two retransmissions, and 30 s absolute
deadline. The receiver ACKs only after validation and admission to its bounded
complete-message queue. Valid duplicates are ACKed again and never emitted
twice. Message-ID reuse with different content is a protocol error.

