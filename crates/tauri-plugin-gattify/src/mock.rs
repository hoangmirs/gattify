use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use parking_lot::Mutex;

use crate::{
    validate_server_definition, AdapterState, AdvertisingReport, Backend, BleError, BleResult,
    Capabilities, Command, ConnectionId, ErrorCode, LinkLimits, OperationContext, OwnerId,
    PermissionOutcome, PermissionState, Reply, ResourceId, ResourceSnapshot, ScanId, ServerId,
    SubscriptionId, Support,
};

#[derive(Default)]
struct MockState {
    next_resource: u64,
    scans: HashMap<ScanId, OwnerId>,
    connections: HashMap<ConnectionId, OwnerId>,
    subscriptions: HashMap<SubscriptionId, (OwnerId, ConnectionId)>,
    servers: HashMap<ServerId, OwnerId>,
    advertising: HashSet<ServerId>,
    cancelled: HashSet<String>,
}

impl MockState {
    fn allocate(&mut self, prefix: &str) -> String {
        self.next_resource += 1;
        format!("{prefix}-{}", self.next_resource)
    }

    fn require_owner<T>(
        map: &HashMap<T, OwnerId>,
        id: &T,
        owner: &OwnerId,
        resource: ResourceId,
    ) -> BleResult<()>
    where
        T: Eq + std::hash::Hash,
    {
        match map.get(id) {
            Some(actual) if actual == owner => Ok(()),
            _ => Err(BleError::invalid_handle(resource)),
        }
    }
}

/// Deterministic backend for unit tests and example development.
///
/// It is intentionally never selected by the production plugin constructor.
#[derive(Default)]
pub struct MockBackend {
    state: Mutex<MockState>,
}

#[async_trait]
impl Backend for MockBackend {
    // A single exhaustive dispatcher keeps mock behavior aligned with Command.
    #[allow(clippy::too_many_lines)]
    async fn execute(&self, context: OperationContext, command: Command) -> BleResult<Reply> {
        let mut state = self.state.lock();
        if state.cancelled.contains(context.operation_id.as_str()) {
            return Err(BleError::new(
                ErrorCode::Cancelled,
                "operation was cancelled",
            ));
        }

        match command {
            Command::GetState => Ok(Reply::State(AdapterState::PoweredOn)),
            Command::GetCapabilities => Ok(Reply::Capabilities(Capabilities {
                central: Support::supported(),
                peripheral: Support::supported(),
                advertising: Support::supported(),
                targeted_notify: Support::supported(),
                simultaneous_roles: Support::supported(),
                background: Support::unsupported("mockForegroundOnly"),
                max_connections: Some(8),
                max_advertising_data_length: Some(31),
            })),
            Command::CheckPermissions | Command::RequestPermissions(_) => {
                Ok(Reply::Permissions(PermissionState {
                    scan: PermissionOutcome::Granted,
                    connect: PermissionOutcome::Granted,
                    advertise: PermissionOutcome::Granted,
                }))
            }
            Command::StartScan(_) => {
                let scan_id = ScanId::new(state.allocate("scan"));
                state.scans.insert(scan_id.clone(), context.owner_id);
                Ok(Reply::ScanStarted { scan_id })
            }
            Command::StopScan { scan_id } => {
                MockState::require_owner(
                    &state.scans,
                    &scan_id,
                    &context.owner_id,
                    scan_id.clone().into(),
                )?;
                state.scans.remove(&scan_id);
                Ok(Reply::Empty)
            }
            Command::Connect { .. } => {
                let connection_id = ConnectionId::new(state.allocate("connection"));
                state
                    .connections
                    .insert(connection_id.clone(), context.owner_id);
                Ok(Reply::Connected {
                    connection_id,
                    limits: LinkLimits {
                        write_with_response: Some(20),
                        write_without_response: Some(20),
                        notification: Some(20),
                        att_mtu: None,
                    },
                })
            }
            Command::Disconnect { connection_id } => {
                MockState::require_owner(
                    &state.connections,
                    &connection_id,
                    &context.owner_id,
                    connection_id.clone().into(),
                )?;
                state.connections.remove(&connection_id);
                state
                    .subscriptions
                    .retain(|_, (_, parent)| parent != &connection_id);
                Ok(Reply::Empty)
            }
            Command::DiscoverServices { connection_id } => {
                MockState::require_owner(
                    &state.connections,
                    &connection_id,
                    &context.owner_id,
                    connection_id.clone().into(),
                )?;
                Ok(Reply::Services(Vec::new()))
            }
            Command::Read { connection_id, .. } => {
                MockState::require_owner(
                    &state.connections,
                    &connection_id,
                    &context.owner_id,
                    connection_id.clone().into(),
                )?;
                Ok(Reply::Bytes {
                    value_base64: String::new(),
                })
            }
            Command::Write {
                connection_id,
                value_base64,
                ..
            } => {
                MockState::require_owner(
                    &state.connections,
                    &connection_id,
                    &context.owner_id,
                    connection_id.clone().into(),
                )?;
                BASE64.decode(value_base64).map_err(|_| {
                    BleError::new(ErrorCode::InvalidArgument, "invalid base64 payload")
                })?;
                Ok(Reply::Empty)
            }
            Command::Subscribe { connection_id, .. } => {
                MockState::require_owner(
                    &state.connections,
                    &connection_id,
                    &context.owner_id,
                    connection_id.clone().into(),
                )?;
                let subscription_id = SubscriptionId::new(state.allocate("subscription"));
                state
                    .subscriptions
                    .insert(subscription_id.clone(), (context.owner_id, connection_id));
                Ok(Reply::SubscriptionStarted { subscription_id })
            }
            Command::Unsubscribe { subscription_id } => {
                let owned = state
                    .subscriptions
                    .get(&subscription_id)
                    .is_some_and(|(owner, _)| owner == &context.owner_id);
                if !owned {
                    return Err(BleError::invalid_handle(subscription_id));
                }
                state.subscriptions.remove(&subscription_id);
                Ok(Reply::Empty)
            }
            Command::CreateServer(definition) => {
                validate_server_definition(&definition)?;
                let server_id = ServerId::new(state.allocate("server"));
                state.servers.insert(server_id.clone(), context.owner_id);
                Ok(Reply::ServerCreated { server_id })
            }
            Command::CloseServer { server_id } => {
                MockState::require_owner(
                    &state.servers,
                    &server_id,
                    &context.owner_id,
                    server_id.clone().into(),
                )?;
                state.advertising.remove(&server_id);
                state.servers.remove(&server_id);
                Ok(Reply::Empty)
            }
            Command::StartAdvertising { server_id, options } => {
                MockState::require_owner(
                    &state.servers,
                    &server_id,
                    &context.owner_id,
                    server_id.clone().into(),
                )?;
                crate::validate_uuid(&options.service_uuid)?;
                state.advertising.insert(server_id);
                Ok(Reply::AdvertisingStarted(AdvertisingReport {
                    local_name_included: options.local_name.is_some(),
                    local_name_truncated: false,
                }))
            }
            Command::StopAdvertising { server_id } => {
                MockState::require_owner(
                    &state.servers,
                    &server_id,
                    &context.owner_id,
                    server_id.clone().into(),
                )?;
                state.advertising.remove(&server_id);
                Ok(Reply::Empty)
            }
            Command::SetValue { server_id, .. } | Command::Notify { server_id, .. } => {
                MockState::require_owner(
                    &state.servers,
                    &server_id,
                    &context.owner_id,
                    server_id.clone().into(),
                )?;
                Ok(Reply::Empty)
            }
            Command::Cancel { operation_id } => {
                state.cancelled.insert(operation_id.0);
                Ok(Reply::Empty)
            }
            Command::CloseOwner => {
                let owner = context.owner_id;
                state.scans.retain(|_, value| value != &owner);
                state.connections.retain(|_, value| value != &owner);
                state.subscriptions.retain(|_, (value, _)| value != &owner);
                let removed_servers: Vec<_> = state
                    .servers
                    .iter()
                    .filter_map(|(id, value)| (value == &owner).then_some(id.clone()))
                    .collect();
                for server_id in removed_servers {
                    state.servers.remove(&server_id);
                    state.advertising.remove(&server_id);
                }
                Ok(Reply::Empty)
            }
            Command::DebugResources => Ok(Reply::Resources(ResourceSnapshot {
                scans: state.scans.len(),
                connections: state.connections.len(),
                subscriptions: state.subscriptions.len(),
                servers: state.servers.len(),
            })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ConnectOptions, DeviceId, Manager, ScanOptions};

    #[test]
    fn owner_cleanup_releases_every_resource() {
        let manager = Manager::new(MockBackend::default());
        let owner = OwnerId::new("webview-a");
        futures_lite::future::block_on(async {
            manager
                .execute(
                    owner.clone(),
                    Command::StartScan(ScanOptions {
                        service_uuids: Vec::new(),
                        timeout_ms: None,
                    }),
                    None,
                )
                .await
                .unwrap();
            manager
                .execute(
                    owner.clone(),
                    Command::Connect {
                        device_id: DeviceId::new("ephemeral-device"),
                        options: ConnectOptions { timeout_ms: None },
                    },
                    None,
                )
                .await
                .unwrap();
            manager
                .execute(owner.clone(), Command::CloseOwner, None)
                .await
                .unwrap();
            let reply = manager
                .execute(owner, Command::DebugResources, None)
                .await
                .unwrap();
            assert_eq!(
                reply,
                Reply::Resources(ResourceSnapshot {
                    scans: 0,
                    connections: 0,
                    subscriptions: 0,
                    servers: 0,
                })
            );
        });
    }

    #[test]
    fn cross_owner_handle_use_is_rejected() {
        let manager = Manager::new(MockBackend::default());
        futures_lite::future::block_on(async {
            let Reply::ScanStarted { scan_id } = manager
                .execute(
                    OwnerId::new("owner-a"),
                    Command::StartScan(ScanOptions {
                        service_uuids: Vec::new(),
                        timeout_ms: None,
                    }),
                    None,
                )
                .await
                .unwrap()
            else {
                panic!("expected scan handle")
            };
            let error = manager
                .execute(OwnerId::new("owner-b"), Command::StopScan { scan_id }, None)
                .await
                .unwrap_err();
            assert_eq!(error.code, ErrorCode::InvalidHandle);
        });
    }
}
