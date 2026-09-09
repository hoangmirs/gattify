//! Tauri BLE plugin entry point.
//!
//! Platform adapters are capability-gated. Operations with no verified backend
//! return Unsupported; the deterministic mock is available only through the
//! explicit mock feature and is never selected by init.

mod system;

use std::sync::Arc;

use ble_core::{Backend, BleResult, Command, Manager, OwnerId, Reply};
use system::SystemBackend;

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
}

impl Default for BleRuntime<SystemBackend> {
    fn default() -> Self {
        Self::new(SystemBackend)
    }
}

#[cfg(feature = "tauri")]
mod tauri_api {
    use super::{BleRuntime, SystemBackend};
    use ble_core::{BleError, BleResult, Command, OwnerId, Reply};
    use serde::Deserialize;
    use tauri::{
        plugin::{Builder, TauriPlugin},
        Manager as _, Runtime, State, Webview,
    };

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ExecuteRequest {
        command: Command,
        deadline_millis: Option<u64>,
    }

    #[tauri::command]
    async fn execute<R: Runtime>(
        webview: Webview<R>,
        state: State<'_, BleRuntime<SystemBackend>>,
        request: ExecuteRequest,
    ) -> BleResult<Reply> {
        state
            .execute(
                OwnerId::new(format!("webview:{}", webview.label())),
                request.command,
                request.deadline_millis,
            )
            .await
    }

    #[tauri::command]
    async fn get_state<R: Runtime>(
        webview: Webview<R>,
        state: State<'_, BleRuntime<SystemBackend>>,
    ) -> BleResult<Reply> {
        state
            .execute(
                OwnerId::new(format!("webview:{}", webview.label())),
                Command::GetState,
                None,
            )
            .await
    }

    #[tauri::command]
    async fn get_capabilities<R: Runtime>(
        webview: Webview<R>,
        state: State<'_, BleRuntime<SystemBackend>>,
    ) -> BleResult<Reply> {
        state
            .execute(
                OwnerId::new(format!("webview:{}", webview.label())),
                Command::GetCapabilities,
                None,
            )
            .await
    }

    #[tauri::command]
    async fn check_permissions<R: Runtime>(
        webview: Webview<R>,
        state: State<'_, BleRuntime<SystemBackend>>,
    ) -> BleResult<Reply> {
        state
            .execute(
                OwnerId::new(format!("webview:{}", webview.label())),
                Command::CheckPermissions,
                None,
            )
            .await
    }

    #[tauri::command]
    async fn close<R: Runtime>(
        webview: Webview<R>,
        state: State<'_, BleRuntime<SystemBackend>>,
    ) -> BleResult<Reply> {
        state
            .execute(
                OwnerId::new(format!("webview:{}", webview.label())),
                Command::CloseOwner,
                None,
            )
            .await
    }

    fn peer_unavailable() -> BleError {
        #[cfg(feature = "peer")]
        let message = "peer transport is compiled but no verified native backend is available";
        #[cfg(not(feature = "peer"))]
        let message = "peer transport is disabled in this Rust build";
        BleError::unsupported(message)
    }

    #[tauri::command]
    async fn create_endpoint(_options: serde_json::Value) -> BleResult<serde_json::Value> {
        Err(peer_unavailable())
    }

    #[tauri::command]
    async fn dial_peer(_endpoint_id: String, _device_id: String) -> BleResult<serde_json::Value> {
        Err(peer_unavailable())
    }

    #[tauri::command]
    async fn send_peer(
        _peer_id: String,
        _value_base64: String,
        _timeout_ms: u64,
    ) -> BleResult<serde_json::Value> {
        Err(peer_unavailable())
    }

    #[tauri::command]
    async fn close_peer(_peer_id: String) -> BleResult<()> {
        Err(peer_unavailable())
    }

    #[tauri::command]
    async fn close_endpoint(_endpoint_id: String) -> BleResult<()> {
        Err(peer_unavailable())
    }

    #[must_use]
    pub fn init<R: Runtime>() -> TauriPlugin<R> {
        Builder::new("ble")
            .setup(|app, _api| {
                app.manage(BleRuntime::<SystemBackend>::default());
                Ok(())
            })
            .invoke_handler(tauri::generate_handler![
                execute,
                get_state,
                get_capabilities,
                check_permissions,
                close,
                create_endpoint,
                dial_peer,
                send_peer,
                close_peer,
                close_endpoint
            ])
            .build()
    }
}

#[cfg(feature = "tauri")]
pub use tauri_api::init;

#[cfg(feature = "mock")]
pub mod testing {
    pub use ble_core::MockBackend;
}
