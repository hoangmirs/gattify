# gattify Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the three-crate workspace into one Tauri plugin named gattify, wire its Android and iOS native code to Rust, and prove the wiring with a lab app that CI compiles for Android and iOS.

**Architecture:** The crates `ble-core` and `ble-peer` merge into `tauri-plugin-gattify`. The core files move to the crate root and the peer files move to a `peer` module. A new build script generates the permissions and links the native projects. A mobile backend sends every `Command` to a native `execute` command. In this sub-project, the native code answers status commands and rejects radio commands with `unsupported`.

**Tech Stack:** Rust 1.89, Tauri 2.11.5, `tauri-plugin` 2.6.3 (build), Kotlin (Android Gradle Plugin from the app), Swift 5.9, TypeScript 5.9, Vite 7, `@tauri-apps/cli` 2.11.4, GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-09-10-gattify-design.md`, sub-project 1.

## Global Constraints

- Crate `tauri-plugin-gattify`. npm package `tauri-plugin-gattify-api`.
- IPC `plugin:gattify|<command>`. Events `gattify://<name>`. Permissions `gattify:scan`, `gattify:peer` and the others.
- Android class `dev.gattify.plugin.GattifyPlugin`. Swift class `GattifyPlugin`, entry symbol `init_plugin_gattify`.
- Names that stay: `BleError`, `BleResult`, `BleRuntime`, `createBle()`, and the characteristic UUIDs in `docs/protocol.md`.
- Repository URL: `https://github.com/hoangmirs/gattify`.
- Rust 1.89, edition 2021. Tauri 2.11.x. Android minimum API 26, Java 17. iOS minimum 15. Node 22.
- `deny.toml` allows only permissive licenses. Every new dependency must use one of them.
- Do not name any app that uses gattify, or any other BLE library, in files, commit messages or PR text. The controller runs a name check before each commit.
- Commit messages follow the repository style: a sentence-case imperative subject, then a plain body. Add no `Co-Authored-By` line.
- Default to no code comment. Write one only for a constraint that the code cannot show.
- Do not push, and do not open a pull request, until the owner approves it in chat.
- `sed -i ''` in this plan is the macOS form. On Linux, write `sed -i`.

## Before you start

- [ ] Rename the branch, so that its name follows the repository convention:

```bash
git branch -m docs/gattify-design feature/gattify-foundation
git branch --show-current
```

Expected: `feature/gattify-foundation`.

## File structure

| Path | Responsibility | Task |
| --- | --- | --- |
| `Cargo.toml` | Workspace with one member | 1 |
| `crates/tauri-plugin-gattify/Cargo.toml` | The one published crate | 1, 3, 4 |
| `crates/tauri-plugin-gattify/src/lib.rs` | Module list, re-exports, `BleRuntime` | 1, 4 |
| `crates/tauri-plugin-gattify/src/{backend,error,manager,mock,model}.rs` | Moved from `ble-core` | 1 |
| `crates/tauri-plugin-gattify/src/peer/` | Moved from `ble-peer` | 1 |
| `crates/tauri-plugin-gattify/src/system.rs` | Desktop backend that returns `Unsupported` | 1 |
| `crates/tauri-plugin-gattify/src/commands.rs` | Tauri commands and `init()` | 4 |
| `crates/tauri-plugin-gattify/src/mobile.rs` | Native bridge: argument shape, error mapping, `MobileBackend` | 4 |
| `crates/tauri-plugin-gattify/build.rs` | Permissions and native project links | 3 |
| `crates/tauri-plugin-gattify/android/` | Kotlin library in the Tauri template layout | 3, 4 |
| `crates/tauri-plugin-gattify/ios/` | Swift package in the Tauri template layout | 3, 4 |
| `packages/plugin-gattify/` | npm package, moved from `packages/plugin-ble` | 2 |
| `examples/gattify-lab/` | Lab app: Vite frontend and `src-tauri` | 5 |
| `.github/workflows/ci.yml` | Android and iOS compile jobs | 2, 6 |
| `docs/adr/002-gattify.md` | Records the crate layout and platform decision | 1 |

---

### Task 1: Merge the crates into `tauri-plugin-gattify`

**Files:**
- Move: `crates/tauri-plugin-ble/` to `crates/tauri-plugin-gattify/`
- Move: `crates/ble-core/src/{backend,error,manager,mock,model}.rs` to `crates/tauri-plugin-gattify/src/`
- Move: `crates/ble-peer/src/{frame,receiver,sender}.rs` to `crates/tauri-plugin-gattify/src/peer/`
- Move: `crates/ble-peer/src/lib.rs` to `crates/tauri-plugin-gattify/src/peer/mod.rs`
- Delete: `crates/ble-core/`, `crates/ble-peer/`
- Modify: `Cargo.toml`, `crates/tauri-plugin-gattify/Cargo.toml`, `crates/tauri-plugin-gattify/src/lib.rs`, `crates/tauri-plugin-gattify/src/backend.rs`, `crates/tauri-plugin-gattify/src/system.rs`, `crates/tauri-plugin-gattify/src/peer/*.rs`, `fuzz/Cargo.toml`, `fuzz/fuzz_targets/frame_decode.rs`, `docs/adr/001-foundation.md`
- Create: `docs/adr/002-gattify.md`

**Interfaces:**
- Produces: the crate root exports everything `ble-core` exported: `Backend`, `Command`, `Event`, `OperationContext`, `Reply`, `BleError`, `BleResult`, `DeliveryOutcome`, `ErrorCode`, `Manager` and every model type.
- Produces: `tauri_plugin_gattify::peer` exports everything `ble-peer` exported.
- Produces: `tauri_plugin_gattify::MockBackend` with the `mock` feature.
- Produces: `BleRuntime::new(backend: impl Backend) -> BleRuntime`. `BleRuntime` has no type parameter.
- Produces: `impl Backend for std::sync::Arc<dyn Backend>`.
- Produces: the plugin registers as `gattify`.

The core files go to the crate root, not to a `core` module. A module named `core` makes every `core::` path ambiguous with the Rust `core` crate.

- [ ] **Step 1: Record the baseline**

Run: `cargo test --workspace --all-features 2>&1 | grep "^test result"`
Expected: every line reports `ok`. The `passed` counts add up to 23.

- [ ] **Step 2: Move the files**

```bash
git mv crates/tauri-plugin-ble crates/tauri-plugin-gattify
git mv crates/ble-core/src/backend.rs crates/ble-core/src/error.rs crates/ble-core/src/manager.rs crates/ble-core/src/mock.rs crates/ble-core/src/model.rs crates/tauri-plugin-gattify/src/
mkdir crates/tauri-plugin-gattify/src/peer
git mv crates/ble-peer/src/frame.rs crates/ble-peer/src/receiver.rs crates/ble-peer/src/sender.rs crates/tauri-plugin-gattify/src/peer/
git mv crates/ble-peer/src/lib.rs crates/tauri-plugin-gattify/src/peer/mod.rs
git rm -q crates/ble-core/src/lib.rs crates/ble-core/Cargo.toml crates/ble-peer/Cargo.toml
rmdir crates/ble-core/src crates/ble-core crates/ble-peer/src crates/ble-peer
```

- [ ] **Step 3: Replace the workspace manifest**

Write `Cargo.toml`:

```toml
[workspace]
members = ["crates/tauri-plugin-gattify"]
resolver = "2"

[workspace.package]
version = "0.1.0"
edition = "2021"
rust-version = "1.89"
license = "MIT"
repository = "https://github.com/hoangmirs/gattify"

[workspace.dependencies]
async-trait = "0.1.89"
base64 = "0.22.1"
parking_lot = "0.12.4"
serde = { version = "1.0.219", features = ["derive"] }
serde_json = "1.0.143"
tauri = { version = "2.11.5", default-features = false }
thiserror = "2.0.16"

[workspace.lints.rust]
unsafe_code = "forbid"

[workspace.lints.clippy]
all = "warn"
pedantic = "warn"
```

- [ ] **Step 4: Replace the crate manifest**

Write `crates/tauri-plugin-gattify/Cargo.toml`:

```toml
[package]
name = "tauri-plugin-gattify"
description = "Tauri v2 BLE plugin with raw GATT and complete messages between phones"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true

[features]
default = ["tauri"]
tauri = ["dep:tauri"]
mock = []

[dependencies]
async-trait.workspace = true
base64.workspace = true
parking_lot.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
tauri = { workspace = true, optional = true }

[dev-dependencies]
futures-lite = "2.6.1"

[lints]
workspace = true
```

- [ ] **Step 5: Write the peer module root**

Write `crates/tauri-plugin-gattify/src/peer/mod.rs`:

```rust
//! Versioned complete-message transport over constrained GATT values.
//!
//! This module performs no radio I/O. Callers submit the returned frames using a
//! targeted GATT write or notification and feed received values into Receiver.

mod frame;
mod receiver;
mod sender;

pub use frame::{fragment, Frame, FrameKind, HEADER_LEN, PROTOCOL_MAJOR};
pub use receiver::{ReceiveAction, Receiver, ReceiverLimits};
pub use sender::{QueuedMessage, SendAction, SendLimits, Sender};

/// UUIDs for the v1 profile characteristics.
pub const INFO_CHARACTERISTIC_UUID: &str = "b1e10f10-6a2c-4a62-8e9e-2c938fa30101";
pub const RX_CHARACTERISTIC_UUID: &str = "b1e10f10-6a2c-4a62-8e9e-2c938fa30102";
pub const TX_CHARACTERISTIC_UUID: &str = "b1e10f10-6a2c-4a62-8e9e-2c938fa30103";
```

- [ ] **Step 6: Point the imports at the new crate**

```bash
cd crates/tauri-plugin-gattify/src
sed -i '' 's/ble_core::/crate::/g' lib.rs system.rs peer/frame.rs peer/receiver.rs peer/sender.rs
sed -i '' 's/^use crate::{fragment, FrameKind};/use crate::peer::{fragment, FrameKind};/' peer/sender.rs
sed -i '' 's/^use crate::{Frame, FrameKind};/use crate::peer::{Frame, FrameKind};/' peer/receiver.rs
sed -i '' 's/^    use crate::fragment;/    use crate::peer::fragment;/' peer/receiver.rs
cd -
```

Run: `grep -rn "ble_core\|ble_peer" crates/tauri-plugin-gattify/src`
Expected: no output.

- [ ] **Step 7: Replace the head of `lib.rs`**

In `crates/tauri-plugin-gattify/src/lib.rs`, replace everything from the first line through the line `use system::SystemBackend;` with:

```rust
//! Entry point of the gattify Tauri plugin.
//!
//! Platform adapters are capability-gated. Operations with no verified backend
//! return Unsupported; the deterministic mock is available only through the
//! explicit mock feature and is never selected by init.

mod backend;
mod error;
mod manager;
#[cfg(any(test, feature = "mock"))]
mod mock;
mod model;
pub mod peer;
mod system;

use std::sync::Arc;

pub use backend::{Backend, Command, Event, OperationContext, Reply};
pub use error::{BleError, BleResult, DeliveryOutcome, ErrorCode};
pub use manager::Manager;
#[cfg(feature = "mock")]
pub use mock::MockBackend;
pub use model::*;
use system::SystemBackend;
```

At the end of the same file, delete the old re-export module:

```rust
#[cfg(feature = "mock")]
pub mod testing {
    pub use crate::MockBackend;
}
```

- [ ] **Step 8: Run the moved tests**

Run: `cargo test --workspace --no-default-features 2>&1 | grep "^test result"`
Expected: `ok`, and the `passed` counts add up to 23.

Run: `cargo test --workspace --all-features 2>&1 | grep "^test result"`
Expected: `ok`, and the `passed` counts add up to 23.

If the compiler reports an unresolved import, fix that import to `crate::` or `crate::peer::` and run the step again.

- [ ] **Step 9: Write the failing test for the runtime**

Append to `crates/tauri-plugin-gattify/src/lib.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_runs_commands_through_a_type_erased_backend() {
        let runtime: BleRuntime = BleRuntime::new(mock::MockBackend::default());
        let reply = futures_lite::future::block_on(runtime.execute(
            OwnerId::new("webview:main"),
            Command::GetState,
            None,
        ));
        assert_eq!(reply, Ok(Reply::State(AdapterState::PoweredOn)));
    }
}
```

- [ ] **Step 10: Run the test and see it fail**

Run: `cargo test -p tauri-plugin-gattify --no-default-features runtime_runs_commands`
Expected: a compile error: `missing generics for struct BleRuntime`.

- [ ] **Step 11: Remove the type parameter from `BleRuntime`**

Append to `crates/tauri-plugin-gattify/src/backend.rs`, before its `#[cfg(test)]` module:

```rust
#[async_trait]
impl Backend for std::sync::Arc<dyn Backend> {
    async fn execute(&self, context: OperationContext, command: Command) -> BleResult<Reply> {
        self.as_ref().execute(context, command).await
    }
}
```

In `crates/tauri-plugin-gattify/src/lib.rs`, replace this text:

```rust
#[derive(Clone)]
pub struct BleRuntime<B: Backend> {
    manager: Arc<Manager<B>>,
}

impl<B: Backend> BleRuntime<B> {
    #[must_use]
    pub fn new(backend: B) -> Self {
        Self {
            manager: Arc::new(Manager::new(backend)),
        }
    }
```

with:

```rust
#[derive(Clone)]
pub struct BleRuntime {
    manager: Arc<Manager<Arc<dyn Backend>>>,
}

impl BleRuntime {
    #[must_use]
    pub fn new(backend: impl Backend) -> Self {
        let backend: Arc<dyn Backend> = Arc::new(backend);
        Self {
            manager: Arc::new(Manager::new(backend)),
        }
    }
```

Keep the methods `execute`, `execute_with_id` and `cancel` as they are. In the same file, replace `impl Default for BleRuntime<SystemBackend> {` with `impl Default for BleRuntime {`.

Then update the Tauri command module in the same file:

```bash
cd crates/tauri-plugin-gattify/src
sed -i '' 's/BleRuntime<SystemBackend>/BleRuntime/g; s/BleRuntime::<SystemBackend>::default()/BleRuntime::default()/; s/use super::{BleRuntime, SystemBackend};/use super::BleRuntime;/' lib.rs
cd -
```

- [ ] **Step 12: Run the tests and see them pass**

Run: `cargo test --workspace --no-default-features 2>&1 | grep "^test result"`
Expected: `ok`, and the `passed` counts add up to 24.

Run: `cargo test --workspace --all-features 2>&1 | grep "^test result"`
Expected: `ok`, and the `passed` counts add up to 24.

- [ ] **Step 13: Register the plugin as `gattify`**

```bash
sed -i '' 's/Builder::new("ble")/Builder::new("gattify")/' crates/tauri-plugin-gattify/src/lib.rs
grep -n 'Builder::new' crates/tauri-plugin-gattify/src/lib.rs
```

Expected: one line with `Builder::new("gattify")`.

- [ ] **Step 14: Point the fuzz target at the new crate**

Write `fuzz/Cargo.toml`:

```toml
[package]
name = "gattify-fuzz"
version = "0.0.0"
publish = false
edition = "2021"

[package.metadata]
cargo-fuzz = true

[dependencies]
tauri-plugin-gattify = { path = "../crates/tauri-plugin-gattify", default-features = false }
libfuzzer-sys = "0.4.9"

[[bin]]
name = "frame_decode"
path = "fuzz_targets/frame_decode.rs"
test = false
doc = false
bench = false

[workspace]
```

In `fuzz/fuzz_targets/frame_decode.rs`, replace `use ble_peer::Frame;` with `use tauri_plugin_gattify::peer::Frame;`.

Run: `cargo check --manifest-path fuzz/Cargo.toml`
Expected: `Finished`. The first run compiles libFuzzer and takes about a minute.

- [ ] **Step 15: Record the decision**

Write `docs/adr/002-gattify.md`:

```markdown
# ADR-002: gattify crate layout and mobile backends

Status: accepted
Date: 2026-09-10

## Decision

Publish one Rust crate, `tauri-plugin-gattify`. The files of the former `ble-core` crate move to the crate root. The former `ble-peer` crate becomes the `peer` module. The crate root holds the core files because a module named `core` makes `core::` paths ambiguous with the Rust `core` crate.

The modules `backend`, `error`, `manager`, `model` and `peer` contain no Tauri, OS SDK or WebView types. The `tauri` feature is on by default and adds the Tauri commands and the mobile backend. Unit tests and the fuzz target build without that feature.

Android and iOS backends use the Tauri mobile plugin API: Kotlin on Android and Swift on iOS. The native code implements raw GATT commands only. The peer protocol runs in Rust. Desktop builds keep a backend that returns Unsupported for radio operations.

The Android library compiles against SDK 36, as the Tauri Android library does. The minimum stays at API 26. The iOS minimum stays at iOS 15.

## Consequences

One crate is one package to publish and version. Apps depend on `tauri-plugin-gattify` and on the npm package `tauri-plugin-gattify-api`. A desktop radio backend needs a new decision record.

This record replaces the crate layout and the desktop backend plan of ADR-001.
```

In `docs/adr/001-foundation.md`, replace the line `Status: accepted for initial implementation` with:

```markdown
Status: accepted for initial implementation. ADR-002 replaces its crate layout and its desktop backend plan.
```

- [ ] **Step 16: Check format and lints**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets --all-features -- -D warnings`
Expected: no diff and no warning. If `cargo fmt --check` reports a diff, run `cargo fmt --all` and run the step again.

- [ ] **Step 17: Commit**

```bash
git add -A Cargo.toml Cargo.lock crates fuzz docs/adr
git commit -m "Merge the Rust crates into tauri-plugin-gattify" -m "The core contracts move to the crate root and the peer protocol becomes the peer module. BleRuntime now holds a type-erased backend, so desktop and mobile builds share one managed state type. The plugin registers as gattify. ADR-002 records the new layout."
```


---

### Task 2: Rename the npm package, the IPC names and the prose

**Files:**
- Move: `packages/plugin-ble/` to `packages/plugin-gattify/`
- Modify: `packages/plugin-gattify/package.json`, `packages/plugin-gattify/src/{index,peer,wire}.ts`, `packages/plugin-gattify/test/facade.test.mjs`, `packages/plugin-gattify/README.md`
- Modify: `package.json`, `package-lock.json`, `examples/offline-chat/package.json`, `.github/workflows/ci.yml`
- Modify: `README.md`, `CHANGELOG.md`, `tests/conformance/README.md`

**Interfaces:**
- Consumes: the plugin name `gattify` from Task 1.
- Produces: the npm package `tauri-plugin-gattify-api`. It invokes `plugin:gattify|<command>` and listens on `gattify://<name>`.

- [ ] **Step 1: Move the package and rename it**

```bash
git mv packages/plugin-ble packages/plugin-gattify
sed -i '' 's/"name": "tauri-plugin-ble-api"/"name": "tauri-plugin-gattify-api"/; s/Framework-neutral TypeScript facade for tauri-plugin-ble/Framework-neutral TypeScript facade for tauri-plugin-gattify/' packages/plugin-gattify/package.json
sed -i '' 's/"name": "tauri-ble-workspace"/"name": "gattify-workspace"/; s#--workspace packages/plugin-ble#--workspace packages/plugin-gattify#' package.json
sed -i '' 's/"name": "tauri-ble-offline-chat-example"/"name": "gattify-offline-chat-example"/' examples/offline-chat/package.json
sed -i '' 's#packages/plugin-ble#packages/plugin-gattify#g' .github/workflows/ci.yml
npm install
```

Expected: `npm install` finishes and rewrites `package-lock.json` with the new names.

- [ ] **Step 2: Change the test to expect the new names**

```bash
sed -i '' 's/plugin:ble|/plugin:gattify|/g' packages/plugin-gattify/test/facade.test.mjs
```

Append this test to `packages/plugin-gattify/test/facade.test.mjs`:

```js
test("scan listens on the gattify event namespace", async () => {
  const bridge = new FakeBridge();
  const events = [];
  bridge.listen = async (event) => {
    events.push(event);
    return () => {};
  };
  const session = await createBle({ bridge });

  const scan = await session.scan({ serviceUuids: [] });
  await scan.stop();

  assert.deepEqual(events, ["gattify://scan-result"]);
});
```

- [ ] **Step 3: Run the tests and see them fail**

Run: `npm test --workspace packages/plugin-gattify`
Expected: FAIL. The first failure reads `unexpected command plugin:ble|get_state`.

- [ ] **Step 4: Rename the IPC and event names**

```bash
sed -i '' 's/plugin:ble|/plugin:gattify|/g; s#ble://#gattify://#g' packages/plugin-gattify/src/index.ts packages/plugin-gattify/src/peer.ts packages/plugin-gattify/src/wire.ts
grep -rn "plugin:ble\|ble://" packages/plugin-gattify/src packages/plugin-gattify/test
```

Expected: no output from `grep`.

- [ ] **Step 5: Run the tests and see them pass**

Run: `npm test && npm run typecheck`
Expected: every test passes. There are 11 tests, one more than before.

- [ ] **Step 6: Rewrite the prose**

In `README.md`, replace the title, the first paragraph and the `## Packages` section, up to the blank line after the note about the npm package name, with:

```markdown
# gattify

A Tauri v2 BLE plugin for raw GATT operations and optional
complete-message transport.

This repository is an implementation-in-progress. The platform-neutral
contracts, deterministic mock backend, TypeScript facade, and peer wire
protocol are implemented. Android and iOS contain native state/capability
probes. No production radio backend is currently claimed as complete. Every
unimplemented production operation returns an explicit Unsupported error.

## Packages

| Package | Status | Purpose |
| --- | --- | --- |
| tauri-plugin-gattify | API surface implemented; native GATT backends gated | Rust crate: DTOs, errors, ownership, backend contract, mock, peer framing and Tauri commands |
| tauri-plugin-gattify-api | Implemented and tested | Framework-neutral TypeScript handles |

The npm package stays private until the first release.
```

Then run:

```bash
sed -i '' 's/tauri_plugin_ble::init()/tauri_plugin_gattify::init()/; s/from "tauri-plugin-ble-api"/from "tauri-plugin-gattify-api"/' README.md
```

In `README.md`, replace the paragraph that starts with `Tauri capabilities opt into roles` with:

```markdown
Tauri capabilities opt into roles separately with `gattify:scan`,
`gattify:connect`, `gattify:server`, `gattify:advertise`, and `gattify:peer`.
The default `gattify:default` permission exposes status queries and owner
cleanup only. The Rust boundary validates each role-specific command even
after Tauri authorizes it.
```

Write `packages/plugin-gattify/README.md`:

```markdown
# TypeScript facade

The framework-neutral frontend API for the gattify Tauri plugin. The package
stays private until the first release.

Import raw GATT operations from the package root and complete-message peer
operations from tauri-plugin-gattify-api/peer. Importing either module performs
no Bluetooth work.
```

In `tests/conformance/README.md`, replace the line `The deterministic mock exercises the portable lifecycle contract in ble-core.` with:

```markdown
The deterministic mock exercises the portable lifecycle contract in
tauri-plugin-gattify.
```

In `CHANGELOG.md`, add this line as the first bullet under `## 0.1.0 - Unreleased`:

```markdown
- Renamed the project to gattify and merged the Rust crates into `tauri-plugin-gattify`.
```

- [ ] **Step 7: Check that no old name remains**

Run:

```bash
git grep -n -E "plugin:ble\||ble://|tauri-plugin-ble|ble-core|ble-peer|ble_core|ble_peer|plugin-ble|tauri-ble|tauri_plugin_ble|ble:(scan|connect|server|advertise|peer|default|status)" -- . ':!Cargo.lock' ':!package-lock.json' ':!docs/superpowers' ':!docs/adr/001-foundation.md'
```

Expected: no output. ADR-001 keeps its old names because it is a record of a past decision.

- [ ] **Step 8: Commit**

```bash
git add -A packages package.json package-lock.json examples/offline-chat/package.json .github/workflows/ci.yml README.md CHANGELOG.md tests/conformance/README.md
git commit -m "Rename the npm package and the IPC names to gattify" -m "The TypeScript package becomes tauri-plugin-gattify-api. It invokes plugin:gattify commands and listens on gattify events. A new test pins the event namespace. The README drops the two rows for packages that do not exist."
```


---

### Task 3: Add the build script and the Tauri native project layout

**Files:**
- Create: `crates/tauri-plugin-gattify/build.rs`
- Modify: `crates/tauri-plugin-gattify/Cargo.toml`
- Move: `crates/tauri-plugin-gattify/android/ble/build.gradle.kts` to `crates/tauri-plugin-gattify/android/build.gradle.kts`
- Move: `crates/tauri-plugin-gattify/android/ble/src/` to `crates/tauri-plugin-gattify/android/src/`
- Move: `.../android/src/main/java/dev/taurible/plugin/BlePlugin.kt` to `.../android/src/main/java/dev/gattify/plugin/GattifyPlugin.kt`
- Delete: `crates/tauri-plugin-gattify/android/settings.gradle.kts`
- Move: `crates/tauri-plugin-gattify/ios/Sources/BlePlugin.swift` to `crates/tauri-plugin-gattify/ios/Sources/GattifyPlugin.swift`
- Modify: `crates/tauri-plugin-gattify/ios/Package.swift`, `.gitignore`, `THIRD_PARTY_NOTICES.md`
- Regenerate: `crates/tauri-plugin-gattify/permissions/autogenerated/`, `crates/tauri-plugin-gattify/permissions/schemas/schema.json`

**Interfaces:**
- Consumes: the crate from Task 1.
- Produces: `links = "tauri-plugin-gattify"`. The app derives the permission prefix `gattify` from this key.
- Produces: an Android library at `android/`. The app includes it as the Gradle project `:tauri-plugin-gattify`. It depends on `:tauri-android`.
- Produces: a Swift package named `tauri-plugin-gattify`. It depends on `../.tauri/tauri-api`.

Background: `tauri_plugin::Builder::build()` fails if `links` is missing. For an Android target, it copies the Tauri Android library to `android/.tauri/tauri-api`. For an iOS target on a macOS host, it copies the Tauri Swift package to `.tauri/tauri-api` in the crate root and runs `swift build`. The app reads `android_path` as the library module itself, and it never reads the plugin's `settings.gradle`. For these reasons, the nested `android/ble` module and its Maven dependency on `app.tauri:tauri-android` do not work.

- [ ] **Step 1: Add the build script**

Write `crates/tauri-plugin-gattify/build.rs`:

```rust
#[cfg(feature = "tauri")]
const COMMANDS: &[&str] = &[
    "execute_scan",
    "execute_connect",
    "execute_server",
    "execute_advertise",
    "request_scan_permission",
    "request_connect_permission",
    "request_advertise_permission",
    "cancel",
    "get_state",
    "get_capabilities",
    "check_permissions",
    "close",
    "create_endpoint",
    "dial_peer",
    "send_peer",
    "close_peer",
    "close_endpoint",
];

fn main() {
    #[cfg(feature = "tauri")]
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .ios_path("ios")
        .build();
}
```

- [ ] **Step 2: Declare the link key and the build dependency**

In `crates/tauri-plugin-gattify/Cargo.toml`, add `links` after `repository.workspace = true`:

```toml
links = "tauri-plugin-gattify"
```

Replace the `tauri` feature line:

```toml
tauri = ["dep:tauri", "dep:tauri-plugin"]
```

Add these sections after `[dependencies]`:

```toml
[target.'cfg(target_os = "ios")'.dependencies]
tauri = { workspace = true, optional = true, features = ["wry"] }

[build-dependencies]
tauri-plugin = { version = "2.6.3", features = ["build"], optional = true }
```

`register_ios_plugin` exists only when `tauri` has its `wry` feature. The workspace turns off the default features of `tauri`, so the iOS target turns `wry` on again.

- [ ] **Step 3: Move the Android library to the template layout**

```bash
cd crates/tauri-plugin-gattify/android
git mv ble/build.gradle.kts build.gradle.kts
git mv ble/src src
git rm -q settings.gradle.kts
mkdir -p src/main/java/dev/gattify/plugin
git mv src/main/java/dev/taurible/plugin/BlePlugin.kt src/main/java/dev/gattify/plugin/GattifyPlugin.kt
rmdir src/main/java/dev/taurible/plugin src/main/java/dev/taurible ble
sed -i '' 's/^package dev.taurible.plugin$/package dev.gattify.plugin/; s/^class BlePlugin(/class GattifyPlugin(/' src/main/java/dev/gattify/plugin/GattifyPlugin.kt
cd -
```

Write `crates/tauri-plugin-gattify/android/build.gradle.kts`:

```kotlin
plugins {
  id("com.android.library")
  id("org.jetbrains.kotlin.android")
}

android {
  namespace = "dev.gattify.plugin"
  compileSdk = 36

  defaultConfig {
    minSdk = 26
  }

  compileOptions {
    sourceCompatibility = JavaVersion.VERSION_17
    targetCompatibility = JavaVersion.VERSION_17
  }
  kotlinOptions {
    jvmTarget = "17"
  }
}

dependencies {
  implementation(project(":tauri-android"))
}
```

The app's Gradle build sets the versions of both plugins. The plugin file names none.

- [ ] **Step 4: Move the Swift package to the template layout**

```bash
cd crates/tauri-plugin-gattify/ios
git mv Sources/BlePlugin.swift Sources/GattifyPlugin.swift
sed -i '' 's/final class BlePlugin: Plugin/final class GattifyPlugin: Plugin/; s/dev.taurible.plugin.corebluetooth/dev.gattify.plugin.corebluetooth/; s/@_cdecl("init_plugin_ble")/@_cdecl("init_plugin_gattify")/; s/  BlePlugin()/  GattifyPlugin()/' Sources/GattifyPlugin.swift
cd -
```

Write `crates/tauri-plugin-gattify/ios/Package.swift`:

```swift
// swift-tools-version:5.9
import PackageDescription

let package = Package(
  name: "tauri-plugin-gattify",
  platforms: [
    .macOS(.v10_15),
    .iOS(.v15),
  ],
  products: [
    .library(
      name: "tauri-plugin-gattify",
      type: .static,
      targets: ["tauri-plugin-gattify"])
  ],
  dependencies: [
    .package(name: "Tauri", path: "../.tauri/tauri-api")
  ],
  targets: [
    .target(
      name: "tauri-plugin-gattify",
      dependencies: [
        .byName(name: "Tauri")
      ],
      path: "Sources")
  ]
)
```

- [ ] **Step 5: Ignore the generated native folders**

Append to `.gitignore`:

```gitignore
.tauri/
/crates/tauri-plugin-gattify/android/build/
/crates/tauri-plugin-gattify/ios/.build/
```

- [ ] **Step 6: Build, and let the build script write the permissions**

Run: `cargo build -p tauri-plugin-gattify`
Expected: `Finished`.

Run: `ls crates/tauri-plugin-gattify/permissions/autogenerated/commands | wc -l && ls crates/tauri-plugin-gattify/permissions/schemas/schema.json crates/tauri-plugin-gattify/permissions/autogenerated/reference.md`
Expected: `17`, then both paths print.

Run: `git status --short crates/tauri-plugin-gattify/permissions`
Expected: the command files show as modified, and `schemas/` and `reference.md` show as new.

- [ ] **Step 7: Update the notices**

In `THIRD_PARTY_NOTICES.md`, replace the line `- AndroidX Annotation — Apache-2.0.` with `- tauri-plugin (build dependency) — Apache-2.0 OR MIT.`. Replace the line `- tauri-swift — Apache-2.0 OR MIT.` with `- The Tauri Android library and Swift package, linked from the tauri crate — Apache-2.0 OR MIT.`.

- [ ] **Step 8: Run the tests, the lints and the name check**

Run: `cargo test --workspace --all-features 2>&1 | grep "^test result" && cargo test --workspace --no-default-features 2>&1 | grep "^test result"`
Expected: `ok`, with 24 passed in each run.

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets --all-features -- -D warnings`
Expected: no diff and no warning.

Run: `git grep -n -E "dev\.taurible|BlePlugin|init_plugin_ble|TauriPluginBle|tauri-swift" -- crates THIRD_PARTY_NOTICES.md`
Expected: no output.

- [ ] **Step 9: Commit**

```bash
git add -A crates .gitignore THIRD_PARTY_NOTICES.md Cargo.lock
git commit -m "Add the plugin build script and the Tauri native layout" -m "build.rs generates the command permissions and links the Android library and the Swift package. The Android code moves to the template layout, because an app includes the android path as the library module itself. Both projects now link the Tauri API from .tauri/tauri-api. The native classes are renamed to GattifyPlugin."
```


---

### Task 4: Bridge every command to the native `execute` command

**Files:**
- Create: `crates/tauri-plugin-gattify/src/commands.rs` (moved from the `tauri_api` module in `lib.rs`)
- Create: `crates/tauri-plugin-gattify/src/mobile.rs`
- Modify: `crates/tauri-plugin-gattify/src/lib.rs`
- Modify: `crates/tauri-plugin-gattify/android/src/main/java/dev/gattify/plugin/GattifyPlugin.kt`
- Modify: `crates/tauri-plugin-gattify/ios/Sources/GattifyPlugin.swift`

**Interfaces:**
- Consumes: `BleRuntime::new(impl Backend)` from Task 1, and the native layout from Task 3.
- Produces: `mobile::MobileBackend<R>(PluginHandle<R>)`, which implements `Backend`. It exists on Android and iOS with the `tauri` feature.
- Produces: the native command `execute` with the arguments `{ operationId, ownerId, deadlineMillis, command }`. It resolves with a `Reply` in JSON, for example `{ "kind": "state", "payload": "poweredOn" }`. It rejects with `{ code, message }`, where `code` is an `ErrorCode` string such as `unsupported`.
- Produces: the native command `setEventChannel` with the argument `{ channel }`. The native code stores the channel. Rust does not call `setEventChannel` in this sub-project. Sub-project 2 adds the call together with the event router, because nothing reads events before then.
- In this task, the native `execute` answers `getState`, `getCapabilities`, `checkPermissions`, `cancel` and `closeOwner`. It rejects every other command with `unsupported`.

- [ ] **Step 1: Move the Tauri command module to its own file**

In `crates/tauri-plugin-gattify/src/lib.rs`, cut the block `#[cfg(feature = "tauri")] mod tauri_api { ... }` and the line `#[cfg(feature = "tauri")] pub use tauri_api::init;`. Paste the inner body of `mod tauri_api { ... }` into a new file, `crates/tauri-plugin-gattify/src/commands.rs`, one indentation level lower. In that file, replace `use super::BleRuntime;` with `use crate::BleRuntime;`.

Add these lines to `lib.rs`, after `mod system;`:

```rust
#[cfg(feature = "tauri")]
mod commands;
```

Add this line to `lib.rs`, after `use system::SystemBackend;`:

```rust
#[cfg(feature = "tauri")]
pub use commands::init;
```

Run: `cargo test --workspace --all-features 2>&1 | grep "^test result"`
Expected: `ok`, and the `passed` counts add up to 24.

- [ ] **Step 2: Write the failing bridge tests**

Add this line to `lib.rs`, after `mod manager;`:

```rust
#[cfg(any(test, target_os = "android", target_os = "ios"))]
mod mobile;
```

Write `crates/tauri-plugin-gattify/src/mobile.rs`:

```rust
#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{AdapterState, Reply, SupportLevel};

    #[test]
    fn execute_args_match_the_native_wire_shape() {
        let operation_id = OperationId::new("op-1");
        let owner_id = OwnerId::new("webview:main");
        let args = ExecuteArgs {
            operation_id: &operation_id,
            owner_id: &owner_id,
            deadline_millis: Some(5_000),
            command: &Command::GetState,
        };

        assert_eq!(
            serde_json::to_value(&args).unwrap(),
            json!({
                "operationId": "op-1",
                "ownerId": "webview:main",
                "deadlineMillis": 5000,
                "command": { "kind": "getState" }
            })
        );
    }

    #[test]
    fn native_replies_parse_into_reply_values() {
        let state: Reply =
            serde_json::from_value(json!({ "kind": "state", "payload": "poweredOn" })).unwrap();
        assert_eq!(state, Reply::State(AdapterState::PoweredOn));

        let empty: Reply = serde_json::from_value(json!({ "kind": "empty" })).unwrap();
        assert_eq!(empty, Reply::Empty);

        let unknown = json!({ "level": "unknown", "reason": "backendNotImplemented" });
        let reply: Reply = serde_json::from_value(json!({
            "kind": "capabilities",
            "payload": {
                "central": unknown,
                "peripheral": unknown,
                "advertising": unknown,
                "targetedNotify": unknown,
                "simultaneousRoles": unknown,
                "background": { "level": "unsupported", "reason": "foregroundOnlyContract" }
            }
        }))
        .unwrap();
        let Reply::Capabilities(capabilities) = reply else {
            panic!("expected a capabilities reply");
        };
        assert_eq!(capabilities.central.level, SupportLevel::Unknown);
        assert_eq!(capabilities.max_connections, None);
    }

    #[test]
    fn a_known_native_error_code_is_kept() {
        let error = rejected(Some("unsupported"), Some("not implemented yet"));

        assert_eq!(error.code, ErrorCode::Unsupported);
        assert_eq!(error.message, "not implemented yet");
        assert_eq!(error.native_code, None);
    }

    #[test]
    fn an_unknown_native_error_code_becomes_internal() {
        let error = rejected(Some("gattStatus133"), None);

        assert_eq!(error.code, ErrorCode::Internal);
        assert_eq!(error.native_code.as_deref(), Some("gattStatus133"));
    }
}
```

- [ ] **Step 3: Run the tests and see them fail**

Run: `cargo test -p tauri-plugin-gattify --no-default-features mobile::`
Expected: compile errors: `cannot find struct, variant or union type ExecuteArgs` and `cannot find function rejected`.

- [ ] **Step 4: Implement the bridge**

Insert this code at the top of `crates/tauri-plugin-gattify/src/mobile.rs`, above the test module:

```rust
use serde::Serialize;

use crate::{BleError, Command, ErrorCode, OperationId, OwnerId};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExecuteArgs<'a> {
    operation_id: &'a OperationId,
    owner_id: &'a OwnerId,
    deadline_millis: Option<u64>,
    command: &'a Command,
}

fn rejected(code: Option<&str>, message: Option<&str>) -> BleError {
    let known = code.and_then(|code| serde_json::from_value::<ErrorCode>(code.into()).ok());
    let mut error = BleError::new(
        known.unwrap_or(ErrorCode::Internal),
        message.unwrap_or("the native plugin rejected the command"),
    );
    if known.is_none() {
        error.native_code = code.map(str::to_owned);
    }
    error
}

#[cfg(all(feature = "tauri", any(target_os = "android", target_os = "ios")))]
pub(crate) use bridge::MobileBackend;

#[cfg(all(feature = "tauri", any(target_os = "android", target_os = "ios")))]
mod bridge {
    use async_trait::async_trait;
    use tauri::{
        plugin::{mobile::PluginInvokeError, PluginHandle},
        Runtime,
    };

    use super::{rejected, ExecuteArgs};
    use crate::{Backend, BleError, BleResult, Command, ErrorCode, OperationContext, Reply};

    pub(crate) struct MobileBackend<R: Runtime>(pub(crate) PluginHandle<R>);

    #[async_trait]
    impl<R: Runtime> Backend for MobileBackend<R> {
        async fn execute(&self, context: OperationContext, command: Command) -> BleResult<Reply> {
            let args = ExecuteArgs {
                operation_id: &context.operation_id,
                owner_id: &context.owner_id,
                deadline_millis: context.deadline_millis,
                command: &command,
            };
            self.0
                .run_mobile_plugin_async("execute", args)
                .await
                .map_err(|error| match error {
                    PluginInvokeError::InvokeRejected(response) => {
                        rejected(response.code.as_deref(), response.message.as_deref())
                    }
                    other => BleError::new(ErrorCode::Internal, other.to_string()),
                })
        }
    }
}
```

- [ ] **Step 5: Run the tests and see them pass**

Run: `cargo test --workspace --no-default-features 2>&1 | grep "^test result"`
Expected: `ok`, and the `passed` counts add up to 28.

- [ ] **Step 6: Register the native plugins in `init()`**

In `crates/tauri-plugin-gattify/src/commands.rs`, add this item above `pub fn init`:

```rust
#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_gattify);
```

Replace the `.setup(...)` call inside `init()`:

```rust
        .setup(|app, _api| {
            app.manage(BleRuntime::default());
            Ok(())
        })
```

with:

```rust
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            let backend = crate::mobile::MobileBackend(
                api.register_android_plugin("dev.gattify.plugin", "GattifyPlugin")?,
            );
            #[cfg(target_os = "ios")]
            let backend =
                crate::mobile::MobileBackend(api.register_ios_plugin(init_plugin_gattify)?);
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            let backend = {
                let _ = api;
                crate::SystemBackend
            };
            app.manage(BleRuntime::new(backend));
            Ok(())
        })
```

Run: `cargo test --workspace --all-features 2>&1 | grep "^test result" && cargo clippy --workspace --all-targets --all-features -- -D warnings`
Expected: `ok` with 28 passed, and no warning.

- [ ] **Step 7: Check the Android Rust code where the tools allow it**

```bash
rustup target add aarch64-linux-android
cargo check -p tauri-plugin-gattify --target aarch64-linux-android
```

Expected: `Finished`. If the check fails inside a build script of the `tauri` crate or of a C dependency, and not in `tauri-plugin-gattify`, record the error in the task report and continue. The Task 6 CI job is the required check. This Mac cannot build for iOS, because it has no full Xcode, so the Task 6 CI job is the only iOS check.

- [ ] **Step 8: Implement the Android `execute` and `setEventChannel` commands**

Write `crates/tauri-plugin-gattify/android/src/main/java/dev/gattify/plugin/GattifyPlugin.kt`:

```kotlin
package dev.gattify.plugin

import android.Manifest
import android.app.Activity
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothManager
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.Permission
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Channel
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class SetEventChannelArgs {
  lateinit var channel: Channel
}

@TauriPlugin(
  permissions = [
    Permission(
      strings = [
        Manifest.permission.BLUETOOTH_SCAN,
        Manifest.permission.BLUETOOTH_CONNECT,
        Manifest.permission.BLUETOOTH_ADVERTISE,
      ],
      alias = "bleRoles",
    ),
  ],
)
class GattifyPlugin(private val activity: Activity) : Plugin(activity) {
  private var events: Channel? = null

  private val adapter: BluetoothAdapter?
    get() = (activity.getSystemService(Context.BLUETOOTH_SERVICE) as BluetoothManager).adapter

  @Command
  fun setEventChannel(invoke: Invoke) {
    events = invoke.parseArgs(SetEventChannelArgs::class.java).channel
    invoke.resolve()
  }

  @Command
  fun execute(invoke: Invoke) {
    when (val kind = invoke.getArgs().getJSObject("command")?.getString("kind")) {
      "getState" -> invoke.resolve(reply("state").put("payload", state()))
      "getCapabilities" -> invoke.resolve(reply("capabilities").put("payload", capabilities()))
      "checkPermissions" -> invoke.resolve(reply("permissions").put("payload", unknownPermissions()))
      "cancel", "closeOwner" -> invoke.resolve(reply("empty"))
      else -> invoke.reject("the Android backend does not implement $kind yet", "unsupported")
    }
  }

  private fun reply(kind: String): JSObject = JSObject().put("kind", kind)

  private fun state(): String = when {
    adapter == null -> "unavailable"
    Build.VERSION.SDK_INT >= 31 &&
      activity.checkSelfPermission(Manifest.permission.BLUETOOTH_CONNECT) !=
        PackageManager.PERMISSION_GRANTED -> "unauthorized"
    adapter?.isEnabled == true -> "poweredOn"
    else -> "poweredOff"
  }

  private fun capabilities(): JSObject {
    val notImplemented = unknown("backendNotImplemented")
    return JSObject()
      .put("central", notImplemented)
      .put("peripheral", notImplemented)
      .put("advertising", notImplemented)
      .put("targetedNotify", notImplemented)
      .put("simultaneousRoles", notImplemented)
      .put("background", JSObject().put("level", "unsupported").put("reason", "foregroundOnlyContract"))
  }

  private fun unknownPermissions(): JSObject =
    JSObject().put("scan", "unknown").put("connect", "unknown").put("advertise", "unknown")

  private fun unknown(reason: String): JSObject =
    JSObject().put("level", "unknown").put("reason", reason)
}
```

The capabilities report `unknown` on purpose. Sub-project 3 reports real support after it implements the radio commands.

- [ ] **Step 9: Implement the iOS `execute` and `setEventChannel` commands**

Write `crates/tauri-plugin-gattify/ios/Sources/GattifyPlugin.swift`:

```swift
import CoreBluetooth
import Tauri

struct SetEventChannelArgs: Decodable {
  let channel: Channel
}

final class GattifyPlugin: Plugin {
  private var events: Channel?

  @objc public func setEventChannel(_ invoke: Invoke) throws {
    events = try invoke.parseArgs(SetEventChannelArgs.self).channel
    invoke.resolve()
  }

  @objc public func execute(_ invoke: Invoke) throws {
    let command = try invoke.getArgs()["command"] as? JSObject
    let kind = command?["kind"] as? String
    switch kind {
    case "getState":
      invoke.resolve(reply("state", adapterState()))
    case "getCapabilities":
      invoke.resolve(reply("capabilities", capabilities()))
    case "checkPermissions":
      invoke.resolve(
        reply("permissions", ["scan": "unknown", "connect": "unknown", "advertise": "unknown"]))
    case "cancel", "closeOwner":
      invoke.resolve(reply("empty"))
    default:
      invoke.reject(
        "the iOS backend does not implement \(kind ?? "this command") yet", code: "unsupported")
    }
  }

  private func reply(_ kind: String, _ payload: Any? = nil) -> JsonObject {
    var reply: JsonObject = ["kind": kind]
    if let payload {
      reply["payload"] = payload
    }
    return reply
  }

  // Reads the authorization without a manager. Creating a CBCentralManager shows the Bluetooth prompt.
  private func adapterState() -> String {
    switch CBManager.authorization {
    case .denied, .restricted:
      return "unauthorized"
    default:
      return "unknown"
    }
  }

  private func capabilities() -> JsonObject {
    let notImplemented: JsonObject = ["level": "unknown", "reason": "backendNotImplemented"]
    return [
      "central": notImplemented,
      "peripheral": notImplemented,
      "advertising": notImplemented,
      "targetedNotify": notImplemented,
      "simultaneousRoles": notImplemented,
      "background": ["level": "unsupported", "reason": "foregroundOnlyContract"] as JsonObject,
    ]
  }
}

@_cdecl("init_plugin_gattify")
public func initPlugin() -> Plugin {
  GattifyPlugin()
}
```

The file no longer creates CoreBluetooth managers in `load`. Sub-project 4 creates them on the first radio command.

- [ ] **Step 10: Commit**

```bash
git add -A crates
git commit -m "Send every command to the native execute command" -m "On Android and iOS, a mobile backend passes each Command to the native execute command and maps a native rejection to a BleError. The native code answers the status commands and rejects radio commands as unsupported. It also stores the event channel for the event path in sub-project 2. iOS no longer creates CoreBluetooth managers at load, so the app no longer shows the Bluetooth prompt at launch."
```


---

### Task 5: Add the lab app

**Files:**
- Move: `examples/ble-lab/README.md` to `examples/gattify-lab/README.md`
- Create: `examples/gattify-lab/package.json`, `examples/gattify-lab/index.html`, `examples/gattify-lab/vite.config.ts`, `examples/gattify-lab/src/main.ts`, `examples/gattify-lab/app-icon.svg`
- Create: `examples/gattify-lab/src-tauri/{Cargo.toml,build.rs,tauri.conf.json}`, `examples/gattify-lab/src-tauri/src/{lib.rs,main.rs}`, `examples/gattify-lab/src-tauri/capabilities/default.json`
- Generate: `examples/gattify-lab/src-tauri/icons/`, `examples/gattify-lab/src-tauri/Cargo.lock`
- Modify: `.gitignore`, `package-lock.json`, `README.md`

**Interfaces:**
- Consumes: `tauri_plugin_gattify::init()`, the permission `gattify:default`, and `createBle()` from `tauri-plugin-gattify-api`.
- Produces: an app with the identifier `dev.gattify.lab` and the window label `main`. Task 6 builds it for Android.

The lab app grants only `gattify:default` in this sub-project. The spec also lists `gattify:scan`, `gattify:peer` and `gattify:scope`. Sub-project 2 adds those grants, because the scope permission does not exist yet.

- [ ] **Step 1: Move the lab README**

```bash
mkdir -p examples/gattify-lab
git mv examples/ble-lab/README.md examples/gattify-lab/README.md
rmdir examples/ble-lab
```

Write `examples/gattify-lab/README.md`:

```markdown
# gattify lab

This example is the manual conformance harness. It shows adapter state and
capabilities today. Later sub-projects add permissions, scan, host and join,
a chat that sends text to a peer, and the time from send to ACK.

Do not use the mock to record hardware evidence. Each operation should show its
opaque handle, operation deadline, native outcome, and actual link value limits
without logging payload contents.

Run it on the desktop from the repository root:

    npm run build
    npm run tauri --workspace examples/gattify-lab -- dev
```

- [ ] **Step 2: Write the frontend**

Write `examples/gattify-lab/package.json`:

```json
{
  "name": "gattify-lab",
  "private": true,
  "version": "0.0.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "vite build",
    "tauri": "tauri"
  },
  "dependencies": {
    "@tauri-apps/api": "^2.8.0",
    "tauri-plugin-gattify-api": "*"
  },
  "devDependencies": {
    "@tauri-apps/cli": "2.11.4",
    "typescript": "^5.9.2",
    "vite": "^7.1.0"
  }
}
```

Write `examples/gattify-lab/vite.config.ts`:

```ts
import { defineConfig } from "vite";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
});
```

Write `examples/gattify-lab/index.html`:

```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>gattify lab</title>
  </head>
  <body>
    <main>
      <h1>gattify lab</h1>
      <pre id="status">Loading</pre>
    </main>
    <script type="module" src="/src/main.ts"></script>
  </body>
</html>
```

Write `examples/gattify-lab/src/main.ts`:

```ts
import { createBle } from "tauri-plugin-gattify-api";

const status = document.querySelector<HTMLPreElement>("#status")!;

async function show(): Promise<void> {
  const ble = await createBle();
  const report = {
    state: await ble.getState(),
    capabilities: await ble.getCapabilities(),
  };
  status.textContent = JSON.stringify(report, null, 2);
}

show().catch((error: unknown) => {
  status.textContent = error instanceof Error ? error.message : JSON.stringify(error);
});
```

- [ ] **Step 3: Write the Tauri app**

Write `examples/gattify-lab/src-tauri/Cargo.toml`:

```toml
[package]
name = "gattify-lab"
version = "0.0.0"
edition = "2021"
publish = false

[lib]
name = "gattify_lab_lib"
crate-type = ["staticlib", "cdylib", "rlib"]

[build-dependencies]
tauri-build = { version = "2.6.3", features = [] }

[dependencies]
tauri = { version = "2.11.5", features = [] }
tauri-plugin-gattify = { path = "../../../crates/tauri-plugin-gattify" }

[workspace]
```

Write `examples/gattify-lab/src-tauri/build.rs`:

```rust
fn main() {
    tauri_build::build();
}
```

Write `examples/gattify-lab/src-tauri/src/lib.rs`:

```rust
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_gattify::init())
        .run(tauri::generate_context!())
        .expect("error while running the gattify lab");
}
```

Write `examples/gattify-lab/src-tauri/src/main.rs`:

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    gattify_lab_lib::run();
}
```

Write `examples/gattify-lab/src-tauri/tauri.conf.json`:

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "gattify-lab",
  "version": "0.0.0",
  "identifier": "dev.gattify.lab",
  "build": {
    "beforeDevCommand": "npm run dev",
    "devUrl": "http://localhost:1420",
    "beforeBuildCommand": "npm run build",
    "frontendDist": "../dist"
  },
  "app": {
    "windows": [{ "title": "gattify lab", "width": 480, "height": 720 }],
    "security": { "csp": null }
  },
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": [
      "icons/32x32.png",
      "icons/128x128.png",
      "icons/128x128@2x.png",
      "icons/icon.icns",
      "icons/icon.ico"
    ],
    "android": { "minSdkVersion": 26 },
    "iOS": { "minimumSystemVersion": "15.0" }
  }
}
```

The two minimum versions must match the plugin. Tauri's default Android minimum is API 24, and the plugin declares API 26. With the default, the Android manifest merge fails.

Write `examples/gattify-lab/src-tauri/capabilities/default.json`:

```json
{
  "identifier": "default",
  "description": "Status permissions for the lab window",
  "windows": ["main"],
  "permissions": ["core:default", "gattify:default"]
}
```

Append to `.gitignore`:

```gitignore
/examples/gattify-lab/src-tauri/target/
/examples/gattify-lab/src-tauri/gen/
```

- [ ] **Step 4: Install the tools and generate the icons**

Write `examples/gattify-lab/app-icon.svg`:

```xml
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024">
  <rect width="1024" height="1024" rx="224" fill="#1d4ed8"/>
  <path d="M432 256 L640 432 L480 560 L640 688 L432 864 V256 Z M432 560 L320 470 M432 560 L320 650" fill="none" stroke="#ffffff" stroke-width="64" stroke-linecap="round" stroke-linejoin="round"/>
</svg>
```

```bash
npm install
npm run tauri --workspace examples/gattify-lab -- icon app-icon.svg
ls examples/gattify-lab/src-tauri/icons
```

Expected: the folder holds `32x32.png`, `128x128.png`, `128x128@2x.png`, `icon.icns`, `icon.ico` and `icon.png`. If the CLI rejects the SVG file, convert it first with `qlmanage -t -s 1024 -o examples/gattify-lab examples/gattify-lab/app-icon.svg`. Then run the `icon` command on the PNG file that `qlmanage` writes.

- [ ] **Step 5: Build the lab app for the desktop**

```bash
npm run build
npm run tauri --workspace examples/gattify-lab -- build --debug --no-bundle
```

Expected: `Finished`, and the command prints the path of the built binary. `tauri-build` validates the capability file during this build. An unknown permission such as a wrong `gattify:default` prefix fails the build here.

- [ ] **Step 6: Add the lab app to the root README**

In `README.md`, append this paragraph at the end of the `## Development` section:

```markdown
The lab app in examples/gattify-lab shows adapter state and capabilities.
Run it on the desktop with `npm run tauri --workspace examples/gattify-lab -- dev`
after `npm run build`.
```

- [ ] **Step 7: Run the whole check set**

Run: `npm test && npm run typecheck && cargo test --workspace --all-features 2>&1 | grep "^test result"`
Expected: 11 npm tests pass, and `cargo test` reports `ok` with 28 passed.

- [ ] **Step 8: Commit**

```bash
git add -A examples .gitignore package-lock.json README.md
git commit -m "Add the gattify lab app" -m "The lab app is a Tauri app with a plain TypeScript frontend. It shows adapter state and capabilities through the plugin. It replaces the placeholder README in examples/ble-lab. It sets the Android and iOS minimum versions that the plugin needs."
```


---

### Task 6: Compile the native code in CI

**Files:**
- Modify: `.github/workflows/ci.yml`

**Interfaces:**
- Consumes: the lab app from Task 5 and the native layout from Task 3.
- Produces: an `android` job that builds the lab app APK for `aarch64`. This build compiles the Kotlin code and the Rust code for Android.
- Produces: an `ios` job that builds the plugin crate for `aarch64-apple-ios`. The plugin build script runs `swift build` on the Swift package there. This build compiles the Swift code and the Rust code for iOS.

The spec says the iOS job builds the lab app for the simulator. This plan builds the plugin crate for iOS instead. That build compiles the same Swift and Rust code. It needs no xcodegen, no simulator runtime and no code signing, and the official Tauri plugins repository uses the same method. The lab app build for iOS stays a step for the owner in sub-project 4, on a Mac with Xcode.

- [ ] **Step 1: Add the triggers and the two jobs**

In `.github/workflows/ci.yml`, replace the `on:` block with:

```yaml
on:
  push:
  pull_request:
  workflow_dispatch:
```

Append these jobs under `jobs:`:

```yaml
  android:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-java@v4
        with:
          distribution: temurin
          java-version: 17
      - uses: actions/setup-node@v4
        with:
          node-version: 22
          cache: npm
      - uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: 1.89.0
          targets: aarch64-linux-android
      - run: npm ci
      - run: npm run build
      - name: Build the lab app for Android
        run: |
          if [ -n "${ANDROID_NDK_HOME:-}" ]; then export NDK_HOME="$ANDROID_NDK_HOME"; fi
          npm run tauri --workspace examples/gattify-lab -- android init --ci --skip-targets-install
          npm run tauri --workspace examples/gattify-lab -- android build --ci --debug --apk --target aarch64

  ios:
    if: github.event_name != 'push'
    runs-on: macos-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: 1.89.0
          targets: aarch64-apple-ios
      - name: Compile the plugin and its Swift package for iOS
        run: cargo build --locked -p tauri-plugin-gattify --target aarch64-apple-ios
```

On a private repository, GitHub counts each macOS minute as ten minutes. The `if:` line limits the iOS job to pull requests and manual runs. A manual run works only after this workflow reaches the default branch, so the first iOS run comes from a pull request.

The Ubuntu runner sets `ANDROID_NDK_HOME` to its default NDK, and the Tauri CLI reads `NDK_HOME`. Without `NDK_HOME`, the CLI uses the newest NDK in `$ANDROID_HOME/ndk`.

- [ ] **Step 2: Check the workflow file**

Run: `actionlint .github/workflows/ci.yml`
Expected: no output. `actionlint` is installed on this Mac through mise. Fix every error it reports and run the check again.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "Compile the Android and iOS native code in CI" -m "The android job builds the lab app APK, which compiles the Kotlin code and the Rust code for Android. The ios job builds the plugin crate for aarch64-apple-ios, and the plugin build script runs swift build on the Swift package. The ios job runs on pull requests and manual runs only, because macOS minutes cost ten times more on a private repository."
```

- [ ] **Step 4: Ask the owner before any push**

Stop here. Ask the owner in chat for permission to push `feature/gattify-foundation` to `origin` and to open a draft pull request against `develop`. The draft pull request starts the iOS job. Continue only after an explicit yes.

- [ ] **Step 5: Push and open the draft pull request**

```bash
git push -u origin feature/gattify-foundation
gh pr create --draft --base develop --title "Build the gattify foundation" --body-file <(printf '%s\n' "Sub-project 1 of docs/superpowers/specs/2026-09-10-gattify-design.md. The plan is docs/superpowers/plans/2026-09-10-gattify-foundation.md." "" "- One crate, tauri-plugin-gattify, and one npm package, tauri-plugin-gattify-api." "- A build script, the Tauri native layout, and a native execute bridge on Android and iOS." "- A lab app that shows adapter state and capabilities." "- CI jobs that compile the Kotlin code and the Swift code.")
```

- [ ] **Step 6: Read the CI result and fix failures**

Run: `gh pr checks --watch`
Expected: the `rust`, `typescript`, `android` and `ios` checks pass.

For each failed job, run `gh run view --log-failed` and read the first error. Fix the cause in the repository. Commit the fix with a message that names the cause. Push again. These failures are the likely ones:

- `ios` reports `unsafe_code` in `ios_plugin_binding!`: change `unsafe_code = "forbid"` to `unsafe_code = "deny"` in `Cargo.toml`. The macro makes a private function, so a module around it hides the function. In `commands.rs`, replace the `tauri::ios_plugin_binding!(init_plugin_gattify);` item with the code below, and call `api.register_ios_plugin(ios_binding::init_plugin_gattify)`.

```rust
#[cfg(target_os = "ios")]
#[allow(unsafe_code)]
mod ios_binding {
    tauri::swift_rs::swift!(pub(crate) fn init_plugin_gattify() -> *const ::std::ffi::c_void);
}
```

- `ios` or `android` reports that the future of `run_mobile_plugin_async` is not `Send`: in `mobile.rs`, replace the body of `MobileBackend::execute` with the code below. The blocking call waits on a channel, so it must not run on an async worker thread.

```rust
            let handle = self.0.clone();
            let result = tauri::async_runtime::spawn_blocking(move || {
                let args = ExecuteArgs {
                    operation_id: &context.operation_id,
                    owner_id: &context.owner_id,
                    deadline_millis: context.deadline_millis,
                    command: &command,
                };
                handle.run_mobile_plugin::<Reply>("execute", args)
            })
            .await
            .map_err(|error| BleError::new(ErrorCode::Internal, error.to_string()))?;
            result.map_err(|error| match error {
                PluginInvokeError::InvokeRejected(response) => {
                    rejected(response.code.as_deref(), response.message.as_deref())
                }
                other => BleError::new(ErrorCode::Internal, other.to_string()),
            })
```
- `android` reports a Kotlin compile error: fix it in `GattifyPlugin.kt`. The Tauri Android API is in the job's copy of `.tauri/tauri-api`.
- `android` reports a manifest merge error about `minSdkVersion`: check `bundle.android.minSdkVersion` in `tauri.conf.json`.

- [ ] **Step 7: Confirm that the Swift code compiled**

```bash
RUN_ID=$(gh run list --branch feature/gattify-foundation --workflow ci.yml --event pull_request --limit 1 --json databaseId --jq '.[0].databaseId')
gh run view "$RUN_ID" --log | grep -i -E "GattifyPlugin.swift|Compiling tauri-plugin-gattify"
```

Expected: at least one line shows the Swift compile of `GattifyPlugin.swift`. If no line appears, the build script did not run `swift build`. Report this to the owner, because the iOS job then does not prove the Swift code.

---

### Task 7: Record the evidence

**Files:**
- Modify: `IMPLEMENTATION_STATUS.md`, `docs/support-matrix.md`, `CHANGELOG.md`

**Interfaces:**
- Consumes: the CI run URL from Task 6.

- [ ] **Step 1: Run the full local check set**

```bash
cargo fmt --all --check
cargo test --workspace --all-features 2>&1 | grep "^test result"
cargo test --workspace --no-default-features 2>&1 | grep "^test result"
cargo clippy --workspace --all-targets --all-features -- -D warnings
npm test
npm run typecheck
npm pack --workspace packages/plugin-gattify --dry-run
```

Expected: no diff, 28 Rust tests pass in each feature set, no clippy warning, 11 npm tests pass, and `npm pack` lists `tauri-plugin-gattify-api`.

- [ ] **Step 2: Update the status document**

In `IMPLEMENTATION_STATUS.md`, set `Updated: 10 September 2026` to the date of the work.

Add these bullets at the end of `## Delivered`:

```markdown
- gattify rename: one crate, `tauri-plugin-gattify`, and one npm package,
  `tauri-plugin-gattify-api`. ADR-002 records the layout.
- Plugin build script, generated command permissions, and the Tauri native
  layout for the Android library and the Swift package.
- Native bridge on Android and iOS: every command reaches the native
  `execute` command. Status commands answer from native code. Radio commands
  return Unsupported. The native code stores an event channel for later use.
- Lab app skeleton in examples/gattify-lab that shows adapter state and
  capabilities.
```

Replace the `## Tests actually run` list with the counts and the tool versions from Step 1. Get the URL of the passing CI run:

```bash
gh run list --branch feature/gattify-foundation --workflow ci.yml --event pull_request --status success --limit 1 --json url --jq '.[0].url'
```

Add one line with that URL: `- CI compiled the Android lab app and the iOS plugin build: ` followed by the URL.

In `## Not verified in this environment`, keep the lines about this Mac. Add: `- Kotlin and Swift compile in CI only.`

- [ ] **Step 3: Update the support matrix**

In `docs/support-matrix.md`, add a column `Compiled in CI` after `Compiled here`. Set it to `Yes` for Android and iOS, and to `No` for macOS, Windows and Linux. Change the Android and iOS `Native source present` cells to `State probe and execute bridge`.

- [ ] **Step 4: Update the changelog**

In `CHANGELOG.md`, add these bullets under `## 0.1.0 - Unreleased`:

```markdown
- Added the plugin build script and the Tauri native project layout.
- Added a native execute bridge on Android and iOS for status commands.
- Added the gattify lab app and CI jobs that compile the Kotlin and Swift code.
```

- [ ] **Step 5: Commit and push**

```bash
git add IMPLEMENTATION_STATUS.md docs/support-matrix.md CHANGELOG.md
git commit -m "Record the evidence for the gattify foundation" -m "The status document lists what sub-project 1 delivered, the tests that ran, and the CI run that compiled the Android and iOS native code. The support matrix gains a column for CI compilation."
git push
```

Expected: the push updates the draft pull request.

- [ ] **Step 6: Report to the owner**

Tell the owner the pull request URL and the CI result. Name every step that did not run as written, such as the local Android check in Task 4 Step 7. Ask the owner whether to mark the pull request ready for review.
