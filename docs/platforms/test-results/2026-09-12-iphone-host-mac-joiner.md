# iPhone host, Mac joiner

- Date: 12 September 2026, 18:56 to 19:01 (UTC+7)
- Host: iPhone 16 Pro (iPhone17,1), iOS 26.6.1, gattify lab debug build of
  commit 41e791c, installed over the local network with `devicectl`
- Joiner: Mac mini (Mac14,12), macOS 26.6.2, the macOS probe of
  `tests/conformance/macos-probe` (first version: one write queue, value
  limit read at `didConnect`)
- Roles: the iPhone listens with the local name "gattify lab"; the probe scans
  for the lab service `80ff87c3-8e84-4914-aedc-0d6a3ba5534d` and dials
- Distance: the same desk, RSSI -69 dBm
- Duration: 267 s from connect to link loss

## Timeline

Times are seconds since the probe started.

| Time | Event |
| --- | --- |
| 17.085 | The probe found the host. The advertised name was "gattify ", the 8-byte iOS cut of "gattify lab" |
| 17.518 | Connected. The probe read a value limit of 20 bytes |
| 18.233 | Info read: `[1]` |
| 18.351 | TX subscription enabled |
| 18.442 | HELLO_ACK, 91 ms after the subscription. The iPhone showed the peer |
| 18.652 | "hello from the mac" (18 bytes, 3 frames) acknowledged in 209 ms |
| 18.653 | The probe started to write 4 KiB as 683 frames of 20 bytes |
| 22.041 | "Hello" from the iPhone arrived |
| 27.142, 32.183 | "Hello" arrived again: two retransmissions |
| 37.282 | "Hello 2" arrived, then twice more at 42.382 and 47.452 |
| 90.443 | "Oh my zep" arrived once |
| 161.577 | 4 KiB from the iPhone arrived once and was acknowledged |
| 284.119 | The link timed out (CoreBluetooth error 6) |

The 4 KiB message from the probe never received an ACK.

## What passed

- iOS advertising with a cut local name, a scoped scan result, connection.
- The iOS server: Info read, TX subscription, RX writes, notifications.
- The whole path from a native write through the event channel to the Rust
  peer driver, and from the driver back to a notification: the handshake.
- Complete messages in both directions, including a 4 KiB host message.

## What failed, and the fixes

1. **ACK starvation.** The probe queued its ACK behind the 683 frames of its
   own 4 KiB message. The iPhone saw no ACK for more than 5 s and retransmitted
   "Hello" twice. The peer driver had the same single queue. Fixed in 75f7c3b:
   HELLO, HELLO_ACK and ACK frames leave ahead of DATA frames. A MockAir test
   with a 30 ms frame delay reproduces the failure and passes with the fix.
2. **20-byte frames.** The value limit read at `didConnect` was 20 bytes: the
   MTU exchange ends later. 683 frames took longer than the 30 s reassembly
   window of the host, so the 4 KiB message never completed. Fixed in 1160abf:
   an iOS connect waits up to 1 s for the negotiated length.
3. **Link loss at 284 s.** A supervision timeout, most likely the iPhone locking
   or leaving the foreground. gattify is foreground-only.

The next run repeats this with the fixes, then swaps the roles.
