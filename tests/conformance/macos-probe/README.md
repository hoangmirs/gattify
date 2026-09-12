# macOS probe

A CoreBluetooth peer for the Mac that speaks the gattify v1 protocol. It lets
one phone run the lab app against the Mac instead of a second phone.

    tests/conformance/macos-probe/build.sh
    open -W --stdout probe.log --stderr probe.log \
      tests/conformance/macos-probe/build/GattifyProbe.app --args join 120

- `join [seconds]` dials the first device that advertises the lab service,
  runs the handshake, sends a short message and then 4 KiB, answers every DATA
  frame with an ACK, and closes 20 s after the 4 KiB ACK.
- `host [seconds]` advertises the lab service as "mac host", answers HELLO and
  sends a greeting to each joiner.

The first run shows the macOS Bluetooth prompt. Set `PROBE_SIGNING_IDENTITY`
to a development certificate so the permission survives rebuilds.

The probe writes one line per protocol step with the time since start, so the
log is the hardware record. It keeps ACK, HELLO and HELLO_ACK frames ahead of
DATA frames and waits for the MTU exchange before discovery, as gattify does.
It does not retransmit.
