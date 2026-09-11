//! Entry point of the gattify Tauri plugin.
//!
//! Platform adapters are capability-gated. Operations with no verified backend
//! return Unsupported; the deterministic mock is available only through the
//! explicit mock feature and is never selected by init.

mod backend;
#[cfg(feature = "tauri")]
mod commands;
mod error;
mod manager;
#[cfg(any(test, target_os = "android", target_os = "ios"))]
mod mobile;
#[cfg(any(test, feature = "mock"))]
mod mock;
mod model;
pub mod peer;
mod system;

use std::sync::Arc;

pub use backend::{Backend, Command, Event, OperationContext, Reply};
#[cfg(feature = "tauri")]
pub use commands::init;
pub use error::{BleError, BleResult, DeliveryOutcome, ErrorCode};
pub use manager::Manager;
#[cfg(feature = "mock")]
pub use mock::MockBackend;
pub use model::*;
use system::SystemBackend;

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

    /// Executes one owner-scoped BLE command through the configured backend.
    ///
    /// # Errors
    ///
    /// Returns the structured backend error with its generated operation ID.
    pub async fn execute(
        &self,
        owner_id: OwnerId,
        command: Command,
        deadline_millis: Option<u64>,
    ) -> BleResult<Reply> {
        self.manager
            .execute(owner_id, command, deadline_millis)
            .await
    }

    /// Executes an owner-scoped command with a caller-visible operation ID.
    ///
    /// # Errors
    ///
    /// Returns a validation, duplicate-operation, or backend error.
    pub async fn execute_with_id(
        &self,
        owner_id: OwnerId,
        operation_id: OperationId,
        command: Command,
        deadline_millis: Option<u64>,
    ) -> BleResult<Reply> {
        self.manager
            .execute_with_id(owner_id, operation_id, command, deadline_millis)
            .await
    }

    /// Cancels an active operation owned by the caller.
    ///
    /// # Errors
    ///
    /// Returns an ownership or backend cancellation error.
    pub async fn cancel(&self, owner_id: &OwnerId, operation_id: &OperationId) -> BleResult<Reply> {
        self.manager.cancel(owner_id, operation_id).await
    }
}

impl Default for BleRuntime {
    fn default() -> Self {
        Self::new(SystemBackend)
    }
}

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
