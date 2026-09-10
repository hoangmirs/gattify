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

#[cfg(feature = "tauri")]
mod tauri_api {
    use super::BleRuntime;
    use crate::{BleError, BleResult, Command, OperationId, OwnerId, PermissionRequest, Reply};
    use serde::Deserialize;
    use tauri::{
        plugin::{Builder, TauriPlugin},
        Manager as _, Runtime, State, Webview,
    };

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ExecuteRequest {
        operation_id: OperationId,
        command: Command,
        deadline_millis: Option<u64>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct CancelRequest {
        operation_id: OperationId,
    }

    #[derive(Clone, Copy)]
    enum CommandRole {
        Scan,
        Connect,
        Server,
        Advertise,
    }

    fn command_has_role(command: &Command, role: CommandRole) -> bool {
        match role {
            CommandRole::Scan => {
                matches!(command, Command::StartScan(_) | Command::StopScan { .. })
            }
            CommandRole::Connect => matches!(
                command,
                Command::Connect { .. }
                    | Command::Disconnect { .. }
                    | Command::DiscoverServices { .. }
                    | Command::Read { .. }
                    | Command::Write { .. }
                    | Command::Subscribe { .. }
                    | Command::Unsubscribe { .. }
            ),
            CommandRole::Server => matches!(
                command,
                Command::CreateServer(_)
                    | Command::CloseServer { .. }
                    | Command::SetValue { .. }
                    | Command::Notify { .. }
            ),
            CommandRole::Advertise => matches!(
                command,
                Command::StartAdvertising { .. } | Command::StopAdvertising { .. }
            ),
        }
    }

    async fn execute_request<R: Runtime>(
        webview: Webview<R>,
        state: State<'_, BleRuntime>,
        request: ExecuteRequest,
        role: CommandRole,
    ) -> BleResult<Reply> {
        if !command_has_role(&request.command, role) {
            return Err(BleError::new(
                crate::ErrorCode::InvalidArgument,
                "command is not permitted through this role-specific endpoint",
            ));
        }
        state
            .execute_with_id(
                owner(&webview),
                request.operation_id,
                request.command,
                request.deadline_millis,
            )
            .await
    }

    #[tauri::command]
    async fn execute_scan<R: Runtime>(
        webview: Webview<R>,
        state: State<'_, BleRuntime>,
        request: ExecuteRequest,
    ) -> BleResult<Reply> {
        execute_request(webview, state, request, CommandRole::Scan).await
    }

    #[tauri::command]
    async fn execute_connect<R: Runtime>(
        webview: Webview<R>,
        state: State<'_, BleRuntime>,
        request: ExecuteRequest,
    ) -> BleResult<Reply> {
        execute_request(webview, state, request, CommandRole::Connect).await
    }

    #[tauri::command]
    async fn execute_server<R: Runtime>(
        webview: Webview<R>,
        state: State<'_, BleRuntime>,
        request: ExecuteRequest,
    ) -> BleResult<Reply> {
        execute_request(webview, state, request, CommandRole::Server).await
    }

    #[tauri::command]
    async fn execute_advertise<R: Runtime>(
        webview: Webview<R>,
        state: State<'_, BleRuntime>,
        request: ExecuteRequest,
    ) -> BleResult<Reply> {
        execute_request(webview, state, request, CommandRole::Advertise).await
    }

    async fn request_permission<R: Runtime>(
        webview: Webview<R>,
        state: State<'_, BleRuntime>,
        request: PermissionRequest,
    ) -> BleResult<Reply> {
        state
            .execute(owner(&webview), Command::RequestPermissions(request), None)
            .await
    }

    #[tauri::command]
    async fn request_scan_permission<R: Runtime>(
        webview: Webview<R>,
        state: State<'_, BleRuntime>,
    ) -> BleResult<Reply> {
        request_permission(
            webview,
            state,
            PermissionRequest {
                scan: true,
                connect: false,
                advertise: false,
            },
        )
        .await
    }

    #[tauri::command]
    async fn request_connect_permission<R: Runtime>(
        webview: Webview<R>,
        state: State<'_, BleRuntime>,
    ) -> BleResult<Reply> {
        request_permission(
            webview,
            state,
            PermissionRequest {
                scan: false,
                connect: true,
                advertise: false,
            },
        )
        .await
    }

    #[tauri::command]
    async fn request_advertise_permission<R: Runtime>(
        webview: Webview<R>,
        state: State<'_, BleRuntime>,
    ) -> BleResult<Reply> {
        request_permission(
            webview,
            state,
            PermissionRequest {
                scan: false,
                connect: false,
                advertise: true,
            },
        )
        .await
    }

    #[tauri::command]
    async fn cancel<R: Runtime>(
        webview: Webview<R>,
        state: State<'_, BleRuntime>,
        request: CancelRequest,
    ) -> BleResult<Reply> {
        state.cancel(&owner(&webview), &request.operation_id).await
    }

    #[tauri::command]
    async fn get_state<R: Runtime>(
        webview: Webview<R>,
        state: State<'_, BleRuntime>,
    ) -> BleResult<Reply> {
        state
            .execute(owner(&webview), Command::GetState, None)
            .await
    }

    #[tauri::command]
    async fn get_capabilities<R: Runtime>(
        webview: Webview<R>,
        state: State<'_, BleRuntime>,
    ) -> BleResult<Reply> {
        state
            .execute(owner(&webview), Command::GetCapabilities, None)
            .await
    }

    #[tauri::command]
    async fn check_permissions<R: Runtime>(
        webview: Webview<R>,
        state: State<'_, BleRuntime>,
    ) -> BleResult<Reply> {
        state
            .execute(owner(&webview), Command::CheckPermissions, None)
            .await
    }

    #[tauri::command]
    async fn close<R: Runtime>(
        webview: Webview<R>,
        state: State<'_, BleRuntime>,
    ) -> BleResult<Reply> {
        state
            .execute(owner(&webview), Command::CloseOwner, None)
            .await
    }

    fn peer_unavailable() -> BleError {
        BleError::unsupported("no peer-capable native backend is available in this build")
    }

    fn owner<R: Runtime>(webview: &Webview<R>) -> OwnerId {
        OwnerId::new(format!("webview:{}", webview.label()))
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
        Builder::new("gattify")
            .setup(|app, _api| {
                app.manage(BleRuntime::default());
                Ok(())
            })
            .invoke_handler(tauri::generate_handler![
                execute_scan,
                execute_connect,
                execute_server,
                execute_advertise,
                request_scan_permission,
                request_connect_permission,
                request_advertise_permission,
                cancel,
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
