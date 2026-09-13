use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot};

use super::engine::{Engine, Poster};
use crate::{Backend, BleError, BleResult, Command, ErrorCode, EventSink, OperationContext, Reply};

/// The Windows backend of the native bridge contract.
///
/// Plugin setup creates it once, on the main thread and outside any runtime,
/// so the engine gets a thread of its own with a current-thread runtime. The
/// thread lives as long as the process.
pub(crate) struct WindowsBackend {
    poster: Poster,
}

impl WindowsBackend {
    pub(crate) fn new(sink: EventSink) -> Self {
        let (sender, inbox) = mpsc::unbounded_channel();
        let poster = Poster(sender);
        let engine_poster = poster.clone();
        // When the thread or its runtime cannot start, the inbox closes and
        // every command rejects with `unavailable`.
        let _ = std::thread::Builder::new()
            .name("gattify-winrt".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_time()
                    .build();
                if let Ok(runtime) = runtime {
                    runtime.block_on(Engine::new(engine_poster, sink).run(inbox));
                }
            });
        Self { poster }
    }
}

#[async_trait]
impl Backend for WindowsBackend {
    async fn execute(&self, context: OperationContext, command: Command) -> BleResult<Reply> {
        let (reply, answer) = oneshot::channel();
        if !self
            .poster
            .post(move |engine| engine.execute(context, command, reply))
        {
            return Err(stopped());
        }
        answer.await.unwrap_or_else(|_| {
            Err(BleError::new(
                ErrorCode::Internal,
                "the Windows backend dropped the operation",
            ))
        })
    }
}

fn stopped() -> BleError {
    BleError::new(
        ErrorCode::Unavailable,
        "the Windows Bluetooth thread is not running",
    )
}
