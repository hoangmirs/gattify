# Platform support matrix

Compiled and hardware-verified are evidence columns, not implications from a
dependency platform list.

| Platform | Target | Native source present | Compiled here | Compiled in CI | Hardware verified | Raw GATT | Targeted notify | Simultaneous roles |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Android | Yes | Full native bridge | Yes | Yes | No | Implemented | Implemented | Implemented, unverified |
| iOS | Yes | Full native bridge | Yes | Yes | Partly: peripheral role and peer host | Implemented | Implemented, verified with one central | Implemented, unverified |
| macOS | Yes | Boundary only | Yes | No | No | Not implemented | Not implemented | Unknown |
| Windows | Yes | Boundary only | No | No | No | Not implemented | Not implemented | Unknown |
| Linux | Yes | Boundary only | No | Yes | No | Not implemented | Not implemented | Unknown |

Hardware evidence lives in `docs/platforms/test-results/`. Foreground is the
only planned v0.1 lifecycle.

