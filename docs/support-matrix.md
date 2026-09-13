# Platform support matrix

Compiled and hardware-verified are evidence columns, not implications from a
dependency platform list.

| Platform | Target | Native source present | Compiled here | Compiled in CI | Hardware verified | Raw GATT | Targeted notify | Simultaneous roles |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Android | Yes | Full native bridge | Yes | Yes | No | Implemented | Implemented | Implemented, unverified |
| iOS | Yes | Full native bridge | Yes | Yes | Partly: peripheral role and peer host | Implemented | Implemented, verified with one central | Implemented, unverified |
| macOS | Yes | The iOS Swift engine through a C ABI | Yes | Yes | No | Implemented | Implemented | Implemented, unverified |
| Windows | Yes | Full Rust backend on WinRT | Type-checked, not linked | Yes | No | Implemented | Implemented | Implemented, unverified |
| Linux | Yes | Boundary only | No | Yes | No | Not implemented | Not implemented | Unknown |

Hardware evidence lives in `docs/platforms/test-results/`. Foreground is the
only planned v0.1 lifecycle. The macOS and Windows "Compiled in CI" entries
come from the `macos` and `windows` jobs of `.github/workflows/ci.yml`.
