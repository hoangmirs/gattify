use std::{collections::HashMap, sync::Arc, time::Duration};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::{
    ipc::{Channel, GlobalScope},
    plugin::{Builder, TauriPlugin},
    webview::PageLoadEvent,
    AppHandle, Manager as _, RunEvent, Runtime, State, Webview, WindowEvent,
};
use tokio::sync::mpsc;

use crate::{
    events::{webview_owner, EventHub, Router},
    gate::{PageGate, Pass},
    peer::{peer_owner, EndpointOptions, PeerDriver, PeerEmitter, SendReceipt},
    BleError, BleResult, BleRuntime, Command, DeliveryOutcome, DeviceId, ErrorCode, EventSink,
    OperationId, PeerId, PermissionRequest, Reply, ScopeGuard, ServiceScope,
};

const DEFAULT_SEND_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a cleanup waits for the commands of the old page.
const CLEANUP_WAIT: Duration = Duration::from_secs(20);

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

/// One entry of the `gattify:scope` permission.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScopeEntry {
    service_uuid: String,
}

fn service_scope(scope: &GlobalScope<ScopeEntry>) -> ServiceScope {
    ServiceScope::new(
        scope
            .allows()
            .iter()
            .map(|entry| entry.service_uuid.as_str()),
        scope
            .denies()
            .iter()
            .map(|entry| entry.service_uuid.as_str()),
    )
}

/// The [`PageGate`] of each webview.
#[derive(Default)]
struct PageGates {
    gates: Mutex<HashMap<String, Arc<PageGate>>>,
}

impl PageGates {
    fn gate(&self, label: &str) -> Arc<PageGate> {
        self.gates
            .lock()
            .entry(label.to_owned())
            .or_default()
            .clone()
    }

    async fn enter(&self, label: &str) -> Pass {
        self.gate(label).enter().await
    }
}

struct Gattify {
    runtime: BleRuntime,
    driver: PeerDriver,
    guard: Arc<ScopeGuard>,
    hub: Arc<EventHub>,
    gates: PageGates,
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
    state: State<'_, Gattify>,
    scope: GlobalScope<ScopeEntry>,
    request: ExecuteRequest,
    role: CommandRole,
) -> BleResult<Reply> {
    if !command_has_role(&request.command, role) {
        return Err(BleError::new(
            ErrorCode::InvalidArgument,
            "command is not permitted through this role-specific endpoint",
        ));
    }
    let _pass = state.gates.enter(webview.label()).await;
    let owner = webview_owner(webview.label());
    let scope = service_scope(&scope);
    let command = state.guard.authorize(&scope, &owner, request.command)?;
    let reply = state
        .runtime
        .execute_with_id(
            owner.clone(),
            request.operation_id,
            command.clone(),
            request.deadline_millis,
        )
        .await;
    state.guard.observe(&scope, &owner, &command, reply)
}

#[tauri::command]
async fn execute_scan<R: Runtime>(
    webview: Webview<R>,
    state: State<'_, Gattify>,
    scope: GlobalScope<ScopeEntry>,
    request: ExecuteRequest,
) -> BleResult<Reply> {
    execute_request(webview, state, scope, request, CommandRole::Scan).await
}

#[tauri::command]
async fn execute_connect<R: Runtime>(
    webview: Webview<R>,
    state: State<'_, Gattify>,
    scope: GlobalScope<ScopeEntry>,
    request: ExecuteRequest,
) -> BleResult<Reply> {
    execute_request(webview, state, scope, request, CommandRole::Connect).await
}

#[tauri::command]
async fn execute_server<R: Runtime>(
    webview: Webview<R>,
    state: State<'_, Gattify>,
    scope: GlobalScope<ScopeEntry>,
    request: ExecuteRequest,
) -> BleResult<Reply> {
    execute_request(webview, state, scope, request, CommandRole::Server).await
}

#[tauri::command]
async fn execute_advertise<R: Runtime>(
    webview: Webview<R>,
    state: State<'_, Gattify>,
    scope: GlobalScope<ScopeEntry>,
    request: ExecuteRequest,
) -> BleResult<Reply> {
    execute_request(webview, state, scope, request, CommandRole::Advertise).await
}

async fn request_permission<R: Runtime>(
    webview: Webview<R>,
    state: State<'_, Gattify>,
    request: PermissionRequest,
) -> BleResult<Reply> {
    state
        .runtime
        .execute(
            webview_owner(webview.label()),
            Command::RequestPermissions(request),
            None,
        )
        .await
}

#[tauri::command]
async fn request_scan_permission<R: Runtime>(
    webview: Webview<R>,
    state: State<'_, Gattify>,
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
    state: State<'_, Gattify>,
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
    state: State<'_, Gattify>,
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
    state: State<'_, Gattify>,
    request: CancelRequest,
) -> BleResult<Reply> {
    state
        .runtime
        .cancel(&webview_owner(webview.label()), &request.operation_id)
        .await
}

async fn status<R: Runtime>(
    webview: Webview<R>,
    state: State<'_, Gattify>,
    command: Command,
) -> BleResult<Reply> {
    state
        .runtime
        .execute(webview_owner(webview.label()), command, None)
        .await
}

#[tauri::command]
async fn get_state<R: Runtime>(webview: Webview<R>, state: State<'_, Gattify>) -> BleResult<Reply> {
    status(webview, state, Command::GetState).await
}

#[tauri::command]
async fn get_capabilities<R: Runtime>(
    webview: Webview<R>,
    state: State<'_, Gattify>,
) -> BleResult<Reply> {
    status(webview, state, Command::GetCapabilities).await
}

#[tauri::command]
async fn check_permissions<R: Runtime>(
    webview: Webview<R>,
    state: State<'_, Gattify>,
) -> BleResult<Reply> {
    status(webview, state, Command::CheckPermissions).await
}

#[tauri::command]
async fn close<R: Runtime>(webview: Webview<R>, state: State<'_, Gattify>) -> BleResult<Reply> {
    state.guard.forget_owner(&webview_owner(webview.label()));
    status(webview, state, Command::CloseOwner).await
}

/// Registers a channel that receives every event of the calling webview.
#[tauri::command]
// Tauri hands command arguments over by value.
#[allow(clippy::needless_pass_by_value)]
fn listen_events<R: Runtime>(
    webview: Webview<R>,
    state: State<'_, Gattify>,
    channel: Channel<serde_json::Value>,
) {
    state.hub.add(webview.label(), channel);
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EndpointCreated {
    endpoint_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PeerDialed {
    peer_id: PeerId,
}

#[tauri::command]
async fn create_endpoint<R: Runtime>(
    webview: Webview<R>,
    state: State<'_, Gattify>,
    scope: GlobalScope<ScopeEntry>,
    mut options: EndpointOptions,
) -> BleResult<EndpointCreated> {
    let _pass = state.gates.enter(webview.label()).await;
    options.service_uuid = service_scope(&scope).admit(&options.service_uuid)?;
    let endpoint_id = state
        .driver
        .create_endpoint(webview.label(), options)
        .await?;
    Ok(EndpointCreated { endpoint_id })
}

#[tauri::command]
async fn dial_peer<R: Runtime>(
    webview: Webview<R>,
    state: State<'_, Gattify>,
    endpoint_id: String,
    device_id: DeviceId,
) -> BleResult<PeerDialed> {
    let _pass = state.gates.enter(webview.label()).await;
    let peer_id = state
        .driver
        .dial(webview.label(), &endpoint_id, device_id)
        .await?;
    Ok(PeerDialed { peer_id })
}

#[tauri::command]
async fn send_peer<R: Runtime>(
    webview: Webview<R>,
    state: State<'_, Gattify>,
    peer_id: PeerId,
    value_base64: String,
    timeout_ms: Option<u64>,
) -> BleResult<SendReceipt> {
    let bytes = BASE64.decode(value_base64).map_err(|_| {
        let mut error = BleError::new(ErrorCode::InvalidArgument, "valueBase64 is not base64");
        error.delivery = Some(DeliveryOutcome::NotSubmitted);
        error
    })?;
    let timeout = timeout_ms.map_or(DEFAULT_SEND_TIMEOUT, Duration::from_millis);
    let _pass = state.gates.enter(webview.label()).await;
    state
        .driver
        .send(webview.label(), &peer_id, bytes, timeout)
        .await
}

#[tauri::command]
// Tauri hands command arguments over by value.
#[allow(clippy::needless_pass_by_value)]
fn close_peer<R: Runtime>(
    webview: Webview<R>,
    state: State<'_, Gattify>,
    peer_id: PeerId,
) -> BleResult<()> {
    state.driver.close_peer(webview.label(), &peer_id)
}

#[tauri::command]
async fn close_endpoint<R: Runtime>(
    webview: Webview<R>,
    state: State<'_, Gattify>,
    endpoint_id: String,
) -> BleResult<()> {
    let _pass = state.gates.enter(webview.label()).await;
    state
        .driver
        .close_endpoint(webview.label(), &endpoint_id)
        .await
}

/// Releases everything a webview held, when it reloads or closes. A reloaded
/// page must not find a scan or an advertisement of the old page still active.
fn release_webview<R: Runtime>(app: &AppHandle<R>, label: &str) {
    let Some(state) = app.try_state::<Gattify>() else {
        return;
    };
    // Hold back the new page before anything else, so its first command
    // cannot run ahead of the cleanup.
    let gate = state.gates.gate(label);
    gate.begin_cleanup();
    state.hub.remove_label(label);
    // Pending sends and dials of the old page fail at once.
    state.driver.forget_label(label);
    let owner = webview_owner(label);
    state.guard.forget_owner(&owner);

    let runtime = state.runtime.clone();
    let driver = state.driver.clone();
    let guard = state.guard.clone();
    let label = label.to_owned();
    let peer = peer_owner(&label);
    tauri::async_runtime::spawn(async move {
        runtime.cancel_owner(&owner).await;
        runtime.cancel_owner(&peer).await;
        // A native layer that never answers must not block the page for good.
        let _ = tokio::time::timeout(CLEANUP_WAIT, gate.wait_idle()).await;
        // Commands of the old page may have registered resources until now.
        driver.forget_label(&label);
        guard.forget_owner(&owner);
        let _ = runtime.execute(owner, Command::CloseOwner, None).await;
        let _ = runtime.execute(peer, Command::CloseOwner, None).await;
        gate.end_cleanup();
    });
}

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_gattify);

#[must_use]
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("gattify")
        .setup(|app, api| {
            let hub = Arc::new(EventHub::default());
            let guard = Arc::new(ScopeGuard::default());
            let (driver_inbox, inbox) = mpsc::unbounded_channel();
            let router = Router {
                hub: hub.clone(),
                guard: guard.clone(),
                driver_inbox,
            };
            let sink: EventSink = Arc::new(move |owner, event| router.dispatch(owner, event));
            #[cfg(target_os = "android")]
            let backend = crate::mobile::MobileBackend::new(
                api.register_android_plugin("dev.gattify.plugin", "GattifyPlugin")?,
                sink,
            );
            #[cfg(target_os = "ios")]
            let backend = crate::mobile::MobileBackend::new(
                api.register_ios_plugin(init_plugin_gattify)?,
                sink,
            );
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            let backend = {
                let _ = (api, sink);
                crate::SystemBackend
            };
            let runtime = BleRuntime::new(backend);
            let emit_hub = hub.clone();
            let emit: PeerEmitter = Arc::new(move |label, event| {
                emit_hub.emit(
                    label,
                    &format!("gattify://{}", event.name()),
                    &event.payload(),
                );
            });
            let driver = PeerDriver::new(runtime.clone(), emit);
            tauri::async_runtime::spawn(driver.clone().run(inbox));
            app.manage(runtime.clone());
            app.manage(Gattify {
                runtime,
                driver,
                guard,
                hub,
                gates: PageGates::default(),
            });
            Ok(())
        })
        .on_page_load(|webview, payload| {
            if payload.event() == PageLoadEvent::Started {
                release_webview(webview.app_handle(), webview.label());
            }
        })
        .on_event(|app, event| {
            if let RunEvent::WindowEvent {
                label,
                event: WindowEvent::Destroyed,
                ..
            } = event
            {
                release_webview(app, label);
            }
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
            listen_events,
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
