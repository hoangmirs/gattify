//! The macOS backend: the Swift engine of the iOS plugin, which `build.rs` compiles for macOS,
//! called through the C ABI of `macos/Bridge.swift`. The JSON on both sides of that ABI is the
//! one of `docs/native-bridge.md`.

use std::{
    collections::HashMap,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicU64, Ordering},
        Once, OnceLock,
    },
};

use async_trait::async_trait;
use parking_lot::Mutex;
use serde::Deserialize;
use tokio::sync::oneshot;

use crate::{
    native::{rejected, ExecuteArgs},
    Backend, BleError, BleResult, Command, ErrorCode, EventEnvelope, EventSink, OperationContext,
    Reply,
};

type ReplyCallback = extern "C" fn(ticket: u64, resolved: bool, json: *const u8, length: isize);
type EventCallback = extern "C" fn(json: *const u8, length: isize);

extern "C" {
    fn gattify_macos_start(on_reply: ReplyCallback, on_event: EventCallback);
    fn gattify_macos_execute(ticket: u64, json: *const u8, length: isize);
}

/// The Swift engine is one per process, so the state that its callbacks reach is too.
#[derive(Default)]
struct Bridge {
    next_ticket: AtomicU64,
    waiting: Mutex<HashMap<u64, oneshot::Sender<Answer>>>,
    sink: Mutex<Option<EventSink>>,
}

struct Answer {
    resolved: bool,
    json: String,
}

#[derive(Deserialize)]
struct Rejection {
    code: Option<String>,
    message: Option<String>,
}

fn bridge() -> &'static Bridge {
    static BRIDGE: OnceLock<Bridge> = OnceLock::new();
    BRIDGE.get_or_init(Bridge::default)
}

pub(crate) struct MacosBackend;

impl MacosBackend {
    /// Starts the Swift bridge. Its events go to `sink`, which replaces the sink of an earlier
    /// backend, as the native layers keep the latest event channel.
    pub(crate) fn new(sink: EventSink) -> Self {
        *bridge().sink.lock() = Some(sink);
        start();
        Self
    }
}

#[async_trait]
impl Backend for MacosBackend {
    async fn execute(&self, context: OperationContext, command: Command) -> BleResult<Reply> {
        let json = serde_json::to_vec(&ExecuteArgs {
            operation_id: &context.operation_id,
            owner_id: &context.owner_id,
            deadline_millis: context.deadline_millis,
            command: &command,
        })
        .map_err(|error| BleError::new(ErrorCode::Internal, error.to_string()))?;
        let (waiter, answer) = oneshot::channel();
        let ticket = bridge().next_ticket.fetch_add(1, Ordering::Relaxed);
        bridge().waiting.lock().insert(ticket, waiter);
        send(ticket, &json);
        let answer = answer.await.map_err(|_| {
            BleError::new(ErrorCode::Internal, "the macOS bridge dropped the command")
        })?;
        decode(&answer)
    }
}

fn decode(answer: &Answer) -> BleResult<Reply> {
    if answer.resolved {
        return serde_json::from_str(&answer.json).map_err(|error| {
            BleError::new(
                ErrorCode::Internal,
                format!("the macOS bridge sent an invalid reply: {error}"),
            )
        });
    }
    let rejection = serde_json::from_str(&answer.json).unwrap_or(Rejection {
        code: None,
        message: None,
    });
    Err(rejected(
        rejection.code.as_deref(),
        rejection.message.as_deref(),
    ))
}

#[allow(unsafe_code)]
fn start() {
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        // SAFETY: both callbacks have the C signatures that `macos/Bridge.swift` declares, and
        // functions live for the whole process.
        unsafe { gattify_macos_start(on_reply, on_event) };
    });
}

#[allow(unsafe_code)]
fn send(ticket: u64, json: &[u8]) {
    // A slice is never longer than isize::MAX bytes.
    let length = isize::try_from(json.len()).unwrap_or(isize::MAX);
    // SAFETY: the pointer and length describe `json`, which outlives the call, and Swift copies
    // the bytes before it returns.
    unsafe { gattify_macos_execute(ticket, json.as_ptr(), length) };
}

/// Copies the bytes of a callback: Swift frees them when the callback returns.
#[allow(unsafe_code)]
fn copy_text(json: *const u8, length: isize) -> String {
    let Ok(length) = usize::try_from(length) else {
        return String::new();
    };
    if json.is_null() || length == 0 {
        return String::new();
    }
    // SAFETY: Swift passes `length` initialized bytes at `json`, valid until the callback
    // returns, and nothing writes to them meanwhile.
    let bytes = unsafe { std::slice::from_raw_parts(json, length) };
    String::from_utf8_lossy(bytes).into_owned()
}

/// Runs on the Swift queue, so it must not block.
extern "C" fn on_reply(ticket: u64, resolved: bool, json: *const u8, length: isize) {
    let json = copy_text(json, length);
    let waiter = bridge().waiting.lock().remove(&ticket);
    if let Some(waiter) = waiter {
        let _ = waiter.send(Answer { resolved, json });
    }
}

/// Runs on the Swift queue, so it must not block.
extern "C" fn on_event(json: *const u8, length: isize) {
    let json = copy_text(json, length);
    let Ok(envelope) = serde_json::from_str::<EventEnvelope>(&json) else {
        return;
    };
    let sink = bridge().sink.lock().clone();
    if let Some(sink) = sink {
        // A panic must not unwind into Swift.
        let _ = catch_unwind(AssertUnwindSafe(|| {
            sink(envelope.owner_id, envelope.event);
        }));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{
        CharacteristicHandle, ConnectOptions, ConnectionId, DeviceId, OperationId, OwnerId,
        ResourceSnapshot, ScanId, SupportLevel,
    };

    fn backend() -> MacosBackend {
        MacosBackend::new(Arc::new(|_, _| {}))
    }

    async fn run(backend: &MacosBackend, command: Command) -> BleResult<Reply> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let context = OperationContext {
            operation_id: OperationId::new(format!(
                "macos-test-{}",
                NEXT.fetch_add(1, Ordering::Relaxed)
            )),
            owner_id: OwnerId::new("webview:macos-test"),
            deadline_millis: None,
        };
        backend.execute(context, command).await
    }

    // None of these commands creates a CoreBluetooth manager, so none shows the Bluetooth prompt.

    #[tokio::test]
    async fn the_swift_engine_reports_every_role_on_macos() {
        let reply = run(&backend(), Command::GetCapabilities).await.unwrap();

        let Reply::Capabilities(capabilities) = reply else {
            panic!("expected capabilities, got {reply:?}");
        };
        assert_eq!(capabilities.central.level, SupportLevel::Supported);
        assert_eq!(capabilities.peripheral.level, SupportLevel::Supported);
        assert_eq!(capabilities.targeted_notify.level, SupportLevel::Supported);
        assert_eq!(capabilities.background.level, SupportLevel::Unsupported);
    }

    #[tokio::test]
    async fn status_commands_answer_without_a_manager() {
        let backend = backend();

        assert!(matches!(
            run(&backend, Command::GetState).await,
            Ok(Reply::State(_))
        ));
        assert!(matches!(
            run(&backend, Command::CheckPermissions).await,
            Ok(Reply::Permissions(_))
        ));
        assert_eq!(
            run(&backend, Command::DebugResources).await,
            Ok(Reply::Resources(ResourceSnapshot {
                scans: 0,
                connections: 0,
                subscriptions: 0,
                servers: 0,
            }))
        );
    }

    #[tokio::test]
    async fn swift_rejections_keep_their_contract_code() {
        let backend = backend();

        let stop = run(
            &backend,
            Command::StopScan {
                scan_id: ScanId::new("scan-404"),
            },
        )
        .await;
        assert_eq!(stop.unwrap_err().code, ErrorCode::InvalidHandle);

        let connect = run(
            &backend,
            Command::Connect {
                device_id: DeviceId::new("device-404"),
                options: ConnectOptions { timeout_ms: None },
            },
        )
        .await;
        assert_eq!(connect.unwrap_err().code, ErrorCode::InvalidHandle);

        let read = run(
            &backend,
            Command::Read {
                connection_id: ConnectionId::new("connection-404"),
                characteristic: CharacteristicHandle::new("connection-404/characteristic-1"),
            },
        )
        .await;
        assert_eq!(read.unwrap_err().code, ErrorCode::InvalidHandle);
    }

    #[tokio::test]
    async fn cancel_and_close_owner_answer_empty() {
        let backend = backend();

        assert_eq!(
            run(
                &backend,
                Command::Cancel {
                    operation_id: OperationId::new("op-404"),
                },
            )
            .await,
            Ok(Reply::Empty)
        );
        assert_eq!(run(&backend, Command::CloseOwner).await, Ok(Reply::Empty));
        assert_eq!(run(&backend, Command::CloseOwner).await, Ok(Reply::Empty));
    }

    #[test]
    fn a_rejection_without_json_becomes_internal() {
        let error = decode(&Answer {
            resolved: false,
            json: String::new(),
        })
        .unwrap_err();

        assert_eq!(error.code, ErrorCode::Internal);
    }

    #[test]
    fn an_invalid_reply_becomes_internal() {
        let error = decode(&Answer {
            resolved: true,
            json: r#"{"kind":"nope"}"#.into(),
        })
        .unwrap_err();

        assert_eq!(error.code, ErrorCode::Internal);
    }
}
