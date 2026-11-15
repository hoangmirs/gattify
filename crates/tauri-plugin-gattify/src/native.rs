//! The wire shapes that Rust shares with the native layers of `docs/native-bridge.md`: Kotlin
//! and Swift through the Tauri mobile plugin API, and Swift on macOS through a C ABI.

use serde::Serialize;

use crate::{BleError, Command, ErrorCode, OperationId, OwnerId};

/// The arguments of the native `execute` command.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExecuteArgs<'a> {
    pub(crate) operation_id: &'a OperationId,
    pub(crate) owner_id: &'a OwnerId,
    pub(crate) deadline_millis: Option<u64>,
    pub(crate) command: &'a Command,
}

/// The error of a native rejection. An unknown code becomes `internal` and stays as `nativeCode`.
pub(crate) fn rejected(code: Option<&str>, message: Option<&str>) -> BleError {
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

/// Runs a call to the native plugin on its own task, and waits for it. `None` when the task
/// itself failed.
///
/// Tauri sends the native reply into a channel and unwraps that send. A caller that stops
/// waiting, as `tokio::time::timeout` around a CLOSE write does on a lost link, drops the
/// channel. The reply that comes later then panics in a JNI callback that cannot unwind, and
/// Android aborts the app. The task holds the channel until the reply comes.
#[cfg(any(
    test,
    all(feature = "tauri", any(target_os = "android", target_os = "ios"))
))]
pub(crate) async fn outliving_caller<T: Send + 'static>(
    call: impl std::future::Future<Output = T> + Send + 'static,
) -> Option<T> {
    tokio::spawn(call).await.ok()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use super::*;
    use crate::{AdapterState, PermissionOutcome, PermissionState, Reply, SupportLevel};

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

        let permissions: Reply = serde_json::from_value(json!({
            "kind": "permissions",
            "payload": { "scan": "unknown", "connect": "unknown", "advertise": "unknown" }
        }))
        .unwrap();
        assert_eq!(
            permissions,
            Reply::Permissions(PermissionState {
                scan: PermissionOutcome::Unknown,
                connect: PermissionOutcome::Unknown,
                advertise: PermissionOutcome::Unknown,
            })
        );
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

    #[tokio::test]
    async fn a_native_call_keeps_its_reply_channel_after_the_caller_stops_waiting() {
        // Tauri unwraps the send of a native reply. A reply that reached a dropped
        // channel aborted an Android app when a CLOSE write timed out on a lost link.
        let (reply, replied) = tokio::sync::oneshot::channel::<()>();
        let (heard, was_heard) = tokio::sync::oneshot::channel::<bool>();
        let call = outliving_caller(async move {
            let _ = heard.send(replied.await.is_ok());
        });
        assert!(tokio::time::timeout(Duration::from_millis(10), call)
            .await
            .is_err());
        assert!(
            reply.send(()).is_ok(),
            "the late reply still finds its channel"
        );
        assert!(was_heard.await.unwrap());
    }
}
