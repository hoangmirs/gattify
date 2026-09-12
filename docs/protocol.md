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

The adapter buffer ceiling accounts for partial-fragment metadata, partial
payloads, admitted complete messages, and the bounded recent-message cache.
At most 64 partial messages are retained by default. Incomplete reassemblies
expire after 30 seconds, and impossible declarations such as a non-empty
message with more fragments than payload bytes are rejected before allocation.

## Session

The Info characteristic holds one byte: the protocol major version, `0x01`. A
joiner reads it before the handshake and stops with a protocol error for any
other value.

HELLO, HELLO_ACK and CLOSE are header-only frames with message ID 0.

| Step | Joiner | Host |
| --- | --- | --- |
| 1 | Connects, discovers the service, reads Info, subscribes to TX | |
| 2 | Writes HELLO to RX with a response | |
| 3 | | Creates a peer for that central and notifies HELLO_ACK on TX |
| 4 | Treats the peer as ready. Without HELLO_ACK within 10 s, disconnects | |

A second HELLO from a central that has a peer replaces it: the host closes the
old peer as lost and creates a new one. The host ignores every other frame from
a central without a peer. The joiner ignores every frame before HELLO_ACK.

DATA and ACK then flow both ways: the joiner writes to RX with a response, and
the host notifies on TX. Each side keeps one message in flight.

CLOSE ends a peer, and its receiver does not reply. A joiner disconnects after
it sends or receives CLOSE. A host cannot disconnect a central, so it forgets
the peer. A lost link, or a central that unsubscribes from TX, closes the peer
as lost.

## Frame size

The joiner uses the write-with-response limit of the link, and the host uses
the notification size of the central. Both use 20 bytes when the platform
reports no size, and never more than 512 bytes, the longest attribute value.

## ACK timing

The 5 s ACK deadline counts from the moment the last fragment of a message is
written, not from the first. Writing many fragments at a small MTU can take
longer than 5 s. The 30 s absolute deadline counts from the first fragment.
