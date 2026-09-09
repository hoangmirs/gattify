# Release checklist

- [ ] Confirm copyright holder and replace provisional package names.
- [ ] Run formatting, clippy, Rust tests, TypeScript tests and generated-type drift.
- [ ] Generate SPDX or CycloneDX SBOM from exact lockfiles.
- [ ] Run cargo-deny and npm license review; update third-party notices.
- [ ] Compile each claimed target and record toolchain evidence.
- [ ] Run required physical-device matrix and targeted-isolation tests.
- [ ] Package crates and npm tarball; inspect included native sources and licenses.
- [ ] Install raw-only and peer builds into fresh Tauri consumers.
- [ ] Confirm support matrix and changelog contain no unverified claims.
- [ ] Obtain explicit publication authorization.

