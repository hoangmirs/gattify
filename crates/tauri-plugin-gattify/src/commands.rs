use crate::{
    BleError, BleResult, BleRuntime, Command, OperationId, OwnerId, PermissionRequest, Reply,
};
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
async fn close<R: Runtime>(webview: Webview<R>, state: State<'_, BleRuntime>) -> BleResult<Reply> {
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

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_gattify);

#[must_use]
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("gattify")
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AdvertisingOptions, CharacteristicHandle, ConnectOptions, ConnectionId, DeviceId, PeerId,
        ScanId, ScanOptions, ServerDefinition, ServerId, SubscriptionId, WriteType,
    };

    fn expected_role(command: &Command) -> Option<CommandRole> {
        match command {
            Command::StartScan(_) | Command::StopScan { .. } => Some(CommandRole::Scan),
            Command::Connect { .. }
            | Command::Disconnect { .. }
            | Command::DiscoverServices { .. }
            | Command::Read { .. }
            | Command::Write { .. }
            | Command::Subscribe { .. }
            | Command::Unsubscribe { .. } => Some(CommandRole::Connect),
            Command::CreateServer(_)
            | Command::CloseServer { .. }
            | Command::SetValue { .. }
            | Command::Notify { .. } => Some(CommandRole::Server),
            Command::StartAdvertising { .. } | Command::StopAdvertising { .. } => {
                Some(CommandRole::Advertise)
            }
            Command::GetState
            | Command::GetCapabilities
            | Command::CheckPermissions
            | Command::RequestPermissions(_)
            | Command::Cancel { .. }
            | Command::CloseOwner
            | Command::DebugResources => None,
        }
    }

    fn every_command() -> Vec<Command> {
        vec![
            Command::GetState,
            Command::GetCapabilities,
            Command::CheckPermissions,
            Command::RequestPermissions(PermissionRequest {
                scan: true,
                connect: false,
                advertise: false,
            }),
            Command::StartScan(ScanOptions {
                service_uuids: Vec::new(),
                timeout_ms: None,
            }),
            Command::StopScan {
                scan_id: ScanId::new("scan-1"),
            },
            Command::Connect {
                device_id: DeviceId::new("device-1"),
                options: ConnectOptions { timeout_ms: None },
            },
            Command::Disconnect {
                connection_id: ConnectionId::new("connection-1"),
            },
            Command::DiscoverServices {
                connection_id: ConnectionId::new("connection-1"),
            },
            Command::Read {
                connection_id: ConnectionId::new("connection-1"),
                characteristic: CharacteristicHandle::new("characteristic-1"),
            },
            Command::Write {
                connection_id: ConnectionId::new("connection-1"),
                characteristic: CharacteristicHandle::new("characteristic-1"),
                value_base64: "AA==".into(),
                write_type: WriteType::WithResponse,
            },
            Command::Subscribe {
                connection_id: ConnectionId::new("connection-1"),
                characteristic: CharacteristicHandle::new("characteristic-1"),
            },
            Command::Unsubscribe {
                subscription_id: SubscriptionId::new("subscription-1"),
            },
            Command::CreateServer(ServerDefinition {
                services: Vec::new(),
            }),
            Command::CloseServer {
                server_id: ServerId::new("server-1"),
            },
            Command::StartAdvertising {
                server_id: ServerId::new("server-1"),
                options: AdvertisingOptions {
                    service_uuid: "180d".into(),
                    local_name: None,
                    local_name_optional: false,
                },
            },
            Command::StopAdvertising {
                server_id: ServerId::new("server-1"),
            },
            Command::SetValue {
                server_id: ServerId::new("server-1"),
                characteristic_key: "characteristic-1".into(),
                value_base64: "AA==".into(),
            },
            Command::Notify {
                server_id: ServerId::new("server-1"),
                peer_id: PeerId::new("peer-1"),
                characteristic_key: "characteristic-1".into(),
                value_base64: "AA==".into(),
            },
            Command::Cancel {
                operation_id: OperationId::new("operation-1"),
            },
            Command::CloseOwner,
            Command::DebugResources,
        ]
    }

    #[test]
    fn each_command_passes_only_its_own_role_endpoint() {
        let roles = [
            CommandRole::Scan,
            CommandRole::Connect,
            CommandRole::Server,
            CommandRole::Advertise,
        ];
        for command in every_command() {
            for role in roles {
                assert_eq!(
                    command_has_role(&command, role),
                    expected_role(&command) == Some(role),
                    "command {command:?} against role {role:?}"
                );
            }
        }
    }
}
