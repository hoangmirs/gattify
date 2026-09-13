# Third-party notices

No third-party source is copied into this repository.

Direct development dependencies currently declared:

- Tauri and @tauri-apps/api — Apache-2.0 OR MIT.
- tokio — MIT.
- serde, serde_json, async-trait, base64, parking_lot, thiserror, futures-lite —
  MIT OR Apache-2.0.
- TypeScript — Apache-2.0.
- tauri-plugin (build dependency) — Apache-2.0 OR MIT.
- windows, windows-collections, windows-future (Windows only) — MIT OR Apache-2.0.
- The Swift compatibility libraries and compiler-rt, linked statically into a
  macOS app from the Xcode toolchain — Apache-2.0 with the Swift Runtime
  Library Exception, and Apache-2.0 WITH LLVM-exception. The Swift runtime and
  CoreBluetooth come with macOS.
- The Tauri Android library and Swift package, linked from the tauri crate — Apache-2.0 OR MIT.
- junit:junit 4.13.2 (Android unit tests only) — EPL-1.0.
- org.json:json 20260814 (Android unit tests only) — public domain.

## Transitive dependencies

`cargo deny --all-features check licenses` passes against `deny.toml`, and CI
runs it. Every crate in `Cargo.lock` uses a license on its allowlist, except
six that Tauri brings in and gattify uses unmodified:

- cssparser, cssparser-macros, dtoa-short and selectors, through Tauri's build
  tooling, and option-ext, through tauri — MPL-2.0. It covers only the files of
  those crates.
- target-lexicon, a Linux build dependency of Tauri's GTK stack — Apache-2.0
  WITH LLVM-exception.

The npm package `tauri-plugin-gattify-api` has no runtime dependencies. Its
peer dependency `@tauri-apps/api` is Apache-2.0 OR MIT. The npm tooling in
`package-lock.json` is MIT, Apache-2.0, ISC or BSD-3-Clause, except
lightningcss, a Vite build tool under MPL-2.0 that no package ships.

The draft release of 0.1.0 carries CycloneDX SBOMs of the crate and the npm
package; `docs/release-checklist.md` says how to make them.

