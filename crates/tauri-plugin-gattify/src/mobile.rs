use async_trait::async_trait;
use serde::Serialize;
use tauri::{
    ipc::{Channel, InvokeResponseBody},
    plugin::{mobile::PluginInvokeError, PluginHandle},
    Runtime,
};

use crate::{
    native::{outliving_caller, rejected, ExecuteArgs},
    Backend, BleError, BleResult, Command, ErrorCode, EventEnvelope, EventSink, OperationContext,
    Reply,
};

#[derive(Serialize)]
struct SetEventChannel {
    channel: Channel<serde_json::Value>,
}

pub(crate) struct MobileBackend<R: Runtime>(PluginHandle<R>);

impl<R: Runtime> MobileBackend<R> {
    /// Wraps the native plugin and gives it the channel for its events.
    pub(crate) fn new(handle: PluginHandle<R>, sink: EventSink) -> Self {
        let channel = Channel::<serde_json::Value>::new(move |body| {
            if let InvokeResponseBody::Json(json) = body {
                if let Ok(envelope) = serde_json::from_str::<EventEnvelope>(&json) {
                    sink(envelope.owner_id, envelope.event);
                }
            }
            Ok(())
        });
        let native = handle.clone();
        tauri::async_runtime::spawn(async move {
            let _ = native
                .run_mobile_plugin_async::<serde_json::Value>(
                    "setEventChannel",
                    SetEventChannel { channel },
                )
                .await;
        });
        Self(handle)
    }
}

#[async_trait]
impl<R: Runtime> Backend for MobileBackend<R> {
    async fn execute(&self, context: OperationContext, command: Command) -> BleResult<Reply> {
        let args = serde_json::to_value(ExecuteArgs {
            operation_id: &context.operation_id,
            owner_id: &context.owner_id,
            deadline_millis: context.deadline_millis,
            command: &command,
        })
        .map_err(|error| BleError::new(ErrorCode::Internal, error.to_string()))?;
        let native = self.0.clone();
        outliving_caller(async move {
            native
                .run_mobile_plugin_async::<Reply>("execute", args)
                .await
        })
        .await
        .ok_or_else(|| BleError::new(ErrorCode::Internal, "the native call stopped"))?
        .map_err(|error| match error {
            PluginInvokeError::InvokeRejected(response) => {
                rejected(response.code.as_deref(), response.message.as_deref())
            }
            other => BleError::new(ErrorCode::Internal, other.to_string()),
        })
    }
}
