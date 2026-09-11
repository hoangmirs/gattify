# Platform support matrix

Compiled and hardware-verified are evidence columns, not implications from a
dependency platform list.

| Platform | Target | Native source present | Compiled here | Compiled in CI | Hardware verified | Raw GATT | Targeted notify | Simultaneous roles |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Android | Yes | State probe and execute bridge | No | Yes | No | Not implemented | Not implemented | Unknown |
| iOS | Yes | State probe and execute bridge | No | Yes | No | Not implemented | Not implemented | Unknown |
| macOS | Yes | Boundary only | Yes | No | No | Not implemented | Not implemented | Unknown |
| Windows | Yes | Boundary only | No | No | No | Not implemented | Not implemented | Unknown |
| Linux | Yes | Boundary only | No | Yes | No | Not implemented | Not implemented | Unknown |

Peer hosting is unavailable until targeted subscriber delivery is implemented
and proven for a platform. Foreground is the only planned v0.1 lifecycle.

