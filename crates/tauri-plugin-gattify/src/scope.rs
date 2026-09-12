//! The service UUID scope of an app, and the characteristic handles it allows.

use std::collections::{HashMap, HashSet};

use parking_lot::Mutex;

use crate::{
    normalize_uuid, BleError, BleResult, CharacteristicHandle, Command, ConnectionId, ErrorCode,
    OwnerId, Reply,
};

/// The service UUIDs an app lists in its `gattify:scope` permission.
#[derive(Clone, Debug, Default)]
pub struct ServiceScope {
    allowed: HashSet<String>,
}

impl ServiceScope {
    /// Builds the scope from the allowed UUIDs minus the denied ones.
    ///
    /// Entries that are not UUIDs are ignored.
    pub fn new<'a>(
        allow: impl IntoIterator<Item = &'a str>,
        deny: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        let denied: HashSet<String> = deny.into_iter().filter_map(normalize_uuid).collect();
        let allowed = allow
            .into_iter()
            .filter_map(normalize_uuid)
            .filter(|uuid| !denied.contains(uuid))
            .collect();
        Self { allowed }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.allowed.is_empty()
    }

    /// Returns the normalized form of `uuid` when the scope allows it.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::InvalidArgument`] for a value that is not a UUID,
    /// and [`ErrorCode::PermissionDenied`] for a UUID outside the scope.
    pub fn admit(&self, uuid: &str) -> BleResult<String> {
        let normalized = normalize_uuid(uuid)
            .ok_or_else(|| BleError::new(ErrorCode::InvalidArgument, "value is not a UUID"))?;
        if self.allowed.contains(&normalized) {
            Ok(normalized)
        } else {
            Err(denied(format!(
                "service UUID {normalized} is outside the gattify:scope"
            )))
        }
    }

    #[must_use]
    pub fn allows(&self, uuid: &str) -> bool {
        self.admit(uuid).is_ok()
    }
}

/// Enforces the scope on raw GATT commands from a webview.
///
/// It remembers, for each connection, the characteristic handles that a
/// scoped discovery returned. Only those handles may be read, written or
/// subscribed.
#[derive(Default)]
pub struct ScopeGuard {
    handles: Mutex<HashMap<(OwnerId, ConnectionId), HashSet<CharacteristicHandle>>>,
}

impl ScopeGuard {
    /// Checks `command` against the scope and returns it with normalized UUIDs.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::PermissionDenied`] when the command reaches outside
    /// the scope, and [`ErrorCode::InvalidArgument`] for a malformed UUID.
    pub fn authorize(
        &self,
        scope: &ServiceScope,
        owner: &OwnerId,
        command: Command,
    ) -> BleResult<Command> {
        if matches!(
            command,
            Command::StopScan { .. }
                | Command::Disconnect { .. }
                | Command::Unsubscribe { .. }
                | Command::CloseServer { .. }
                | Command::StopAdvertising { .. }
        ) {
            return Ok(command);
        }
        if scope.is_empty() {
            return Err(denied("the gattify:scope permission lists no service UUID"));
        }
        match command {
            Command::StartScan(mut options) => {
                if options.service_uuids.is_empty() {
                    return Err(denied("a scan needs a service UUID filter"));
                }
                options.service_uuids = options
                    .service_uuids
                    .iter()
                    .map(|uuid| scope.admit(uuid))
                    .collect::<BleResult<_>>()?;
                Ok(Command::StartScan(options))
            }
            Command::Read {
                ref connection_id,
                ref characteristic,
            }
            | Command::Write {
                ref connection_id,
                ref characteristic,
                ..
            }
            | Command::Subscribe {
                ref connection_id,
                ref characteristic,
            } => {
                let allowed = self
                    .handles
                    .lock()
                    .get(&(owner.clone(), connection_id.clone()))
                    .is_some_and(|handles| handles.contains(characteristic));
                if allowed {
                    Ok(command)
                } else {
                    Err(denied(
                        "the characteristic did not come from a scoped discovery on this connection",
                    ))
                }
            }
            Command::CreateServer(mut definition) => {
                for service in &mut definition.services {
                    service.uuid = scope.admit(&service.uuid)?;
                    for characteristic in &mut service.characteristics {
                        if let Some(uuid) = normalize_uuid(&characteristic.uuid) {
                            characteristic.uuid = uuid;
                        }
                    }
                }
                Ok(Command::CreateServer(definition))
            }
            Command::StartAdvertising {
                server_id,
                mut options,
            } => {
                options.service_uuid = scope.admit(&options.service_uuid)?;
                Ok(Command::StartAdvertising { server_id, options })
            }
            other => Ok(other),
        }
    }

    /// Applies the scope to the reply of an executed command.
    ///
    /// A discovery reply loses every service outside the scope, and the guard
    /// stores the characteristic handles that remain.
    ///
    /// # Errors
    ///
    /// Returns the error of `reply` unchanged.
    pub fn observe(
        &self,
        scope: &ServiceScope,
        owner: &OwnerId,
        command: &Command,
        reply: BleResult<Reply>,
    ) -> BleResult<Reply> {
        match (command, reply) {
            (Command::DiscoverServices { connection_id }, Ok(Reply::Services(services))) => {
                let services: Vec<_> = services
                    .into_iter()
                    .filter(|service| scope.allows(&service.uuid))
                    .collect();
                let handles = services
                    .iter()
                    .flat_map(|service| &service.characteristics)
                    .map(|characteristic| characteristic.handle.clone())
                    .collect();
                self.handles
                    .lock()
                    .insert((owner.clone(), connection_id.clone()), handles);
                Ok(Reply::Services(services))
            }
            (Command::Disconnect { connection_id }, reply) => {
                self.forget_connection(owner, connection_id);
                reply
            }
            (_, reply) => reply,
        }
    }

    pub fn forget_connection(&self, owner: &OwnerId, connection_id: &ConnectionId) {
        self.handles
            .lock()
            .remove(&(owner.clone(), connection_id.clone()));
    }

    pub fn forget_owner(&self, owner: &OwnerId) {
        self.handles
            .lock()
            .retain(|(handle_owner, _), _| handle_owner != owner);
    }
}

fn denied(message: impl Into<String>) -> BleError {
    BleError::new(ErrorCode::PermissionDenied, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AdvertisingOptions, CharacteristicInstance, CharacteristicProperties, ConnectOptions,
        DeviceId, LocalCharacteristic, LocalService, ScanId, ScanOptions, ServerDefinition,
        ServerId, ServiceHandle, ServiceInstance, SubscriptionId, WriteType,
    };

    const LAB: &str = "80ff87c3-8e84-4914-aedc-0d6a3ba5534d";
    const OTHER: &str = "0000180d-0000-1000-8000-00805f9b34fb";

    fn scope() -> ServiceScope {
        ServiceScope::new([LAB], [])
    }

    fn owner() -> OwnerId {
        OwnerId::new("webview:main")
    }

    fn connection() -> ConnectionId {
        ConnectionId::new("connection-1")
    }

    fn code(result: BleResult<Command>) -> ErrorCode {
        result.unwrap_err().code
    }

    fn scan(uuids: &[&str]) -> Command {
        Command::StartScan(ScanOptions {
            service_uuids: uuids.iter().map(|uuid| (*uuid).to_owned()).collect(),
            timeout_ms: None,
        })
    }

    fn read(handle: &str) -> Command {
        Command::Read {
            connection_id: connection(),
            characteristic: CharacteristicHandle::new(handle),
        }
    }

    fn server(uuid: &str) -> Command {
        Command::CreateServer(ServerDefinition {
            services: vec![LocalService {
                instance_key: "peer".into(),
                uuid: uuid.into(),
                primary: true,
                characteristics: vec![LocalCharacteristic {
                    instance_key: "info".into(),
                    uuid: "B1E10F10-6A2C-4A62-8E9E-2C938FA30101".into(),
                    properties: CharacteristicProperties::default(),
                    initial_value_base64: None,
                    max_value_length: 1,
                }],
            }],
        })
    }

    fn service(uuid: &str, characteristic: &str) -> ServiceInstance {
        ServiceInstance {
            handle: ServiceHandle::new(format!("connection-1/{characteristic}-service")),
            uuid: uuid.into(),
            characteristics: vec![CharacteristicInstance {
                handle: CharacteristicHandle::new(characteristic),
                uuid: "b1e10f10-6a2c-4a62-8e9e-2c938fa30101".into(),
                properties: CharacteristicProperties::default(),
            }],
        }
    }

    fn discover(guard: &ScopeGuard) -> Vec<ServiceInstance> {
        let command = Command::DiscoverServices {
            connection_id: connection(),
        };
        let reply = Ok(Reply::Services(vec![
            service(LAB, "connection-1/characteristic-1"),
            service(OTHER, "connection-1/characteristic-2"),
        ]));
        let Reply::Services(services) = guard.observe(&scope(), &owner(), &command, reply).unwrap()
        else {
            panic!("expected services");
        };
        services
    }

    #[test]
    fn denied_uuids_leave_the_scope() {
        let scope = ServiceScope::new([LAB, OTHER], ["180D"]);
        assert!(scope.allows(LAB));
        assert!(!scope.allows(OTHER));
    }

    #[test]
    fn cleanup_commands_never_read_the_scope() {
        let guard = ScopeGuard::default();
        let empty = ServiceScope::default();
        for command in [
            Command::StopScan {
                scan_id: ScanId::new("scan-1"),
            },
            Command::Disconnect {
                connection_id: connection(),
            },
            Command::Unsubscribe {
                subscription_id: SubscriptionId::new("subscription-1"),
            },
            Command::CloseServer {
                server_id: ServerId::new("server-1"),
            },
            Command::StopAdvertising {
                server_id: ServerId::new("server-1"),
            },
        ] {
            assert_eq!(
                guard.authorize(&empty, &owner(), command.clone()),
                Ok(command)
            );
        }
    }

    #[test]
    fn an_empty_scope_rejects_every_radio_command() {
        let guard = ScopeGuard::default();
        let empty = ServiceScope::default();
        let connect = Command::Connect {
            device_id: DeviceId::new("device-1"),
            options: ConnectOptions { timeout_ms: None },
        };
        assert_eq!(
            code(guard.authorize(&empty, &owner(), connect)),
            ErrorCode::PermissionDenied
        );
        assert_eq!(
            code(guard.authorize(&empty, &owner(), scan(&[LAB]))),
            ErrorCode::PermissionDenied
        );
    }

    #[test]
    fn a_scan_needs_a_filter_inside_the_scope() {
        let guard = ScopeGuard::default();
        assert_eq!(
            code(guard.authorize(&scope(), &owner(), scan(&[]))),
            ErrorCode::PermissionDenied
        );
        assert_eq!(
            code(guard.authorize(&scope(), &owner(), scan(&[LAB, OTHER]))),
            ErrorCode::PermissionDenied
        );
        assert_eq!(
            code(guard.authorize(&scope(), &owner(), scan(&["not-a-uuid"]))),
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            guard.authorize(
                &scope(),
                &owner(),
                scan(&["80FF87C38E844914AEDC0D6A3BA5534D"])
            ),
            Ok(scan(&[LAB]))
        );
    }

    #[test]
    fn connect_is_left_to_the_native_scan_rule() {
        let guard = ScopeGuard::default();
        let connect = Command::Connect {
            device_id: DeviceId::new("device-1"),
            options: ConnectOptions { timeout_ms: None },
        };
        assert_eq!(
            guard.authorize(&scope(), &owner(), connect.clone()),
            Ok(connect)
        );
    }

    #[test]
    fn discovery_drops_services_outside_the_scope() {
        let guard = ScopeGuard::default();
        let services = discover(&guard);
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].uuid, LAB);
    }

    #[test]
    fn only_handles_from_a_scoped_discovery_are_usable() {
        let guard = ScopeGuard::default();
        assert_eq!(
            code(guard.authorize(&scope(), &owner(), read("connection-1/characteristic-1"))),
            ErrorCode::PermissionDenied
        );
        discover(&guard);
        let allowed = read("connection-1/characteristic-1");
        assert_eq!(
            guard.authorize(&scope(), &owner(), allowed.clone()),
            Ok(allowed)
        );
        assert_eq!(
            code(guard.authorize(&scope(), &owner(), read("connection-1/characteristic-2"))),
            ErrorCode::PermissionDenied
        );
        let write = Command::Write {
            connection_id: connection(),
            characteristic: CharacteristicHandle::new("connection-1/characteristic-1"),
            value_base64: "AA==".into(),
            write_type: WriteType::WithResponse,
        };
        assert!(guard.authorize(&scope(), &owner(), write).is_ok());
        let subscribe = Command::Subscribe {
            connection_id: ConnectionId::new("connection-9"),
            characteristic: CharacteristicHandle::new("connection-1/characteristic-1"),
        };
        assert_eq!(
            code(guard.authorize(&scope(), &owner(), subscribe)),
            ErrorCode::PermissionDenied
        );
        assert_eq!(
            code(guard.authorize(
                &scope(),
                &OwnerId::new("webview:other"),
                read("connection-1/characteristic-1")
            )),
            ErrorCode::PermissionDenied
        );
    }

    #[test]
    fn handles_are_forgotten_with_their_connection() {
        let guard = ScopeGuard::default();
        discover(&guard);
        let disconnect = Command::Disconnect {
            connection_id: connection(),
        };
        guard
            .observe(&scope(), &owner(), &disconnect, Ok(Reply::Empty))
            .unwrap();
        assert!(guard
            .authorize(&scope(), &owner(), read("connection-1/characteristic-1"))
            .is_err());

        discover(&guard);
        guard.forget_connection(&owner(), &connection());
        assert!(guard
            .authorize(&scope(), &owner(), read("connection-1/characteristic-1"))
            .is_err());

        discover(&guard);
        guard.forget_owner(&owner());
        assert!(guard
            .authorize(&scope(), &owner(), read("connection-1/characteristic-1"))
            .is_err());
    }

    #[test]
    fn a_server_may_only_host_scoped_services() {
        let guard = ScopeGuard::default();
        assert_eq!(
            code(guard.authorize(&scope(), &owner(), server(OTHER))),
            ErrorCode::PermissionDenied
        );
        let Command::CreateServer(definition) = guard
            .authorize(
                &scope(),
                &owner(),
                server("80FF87C38E844914AEDC0D6A3BA5534D"),
            )
            .unwrap()
        else {
            panic!("expected a server definition");
        };
        assert_eq!(definition.services[0].uuid, LAB);
        assert_eq!(
            definition.services[0].characteristics[0].uuid,
            "b1e10f10-6a2c-4a62-8e9e-2c938fa30101"
        );
    }

    #[test]
    fn advertising_is_limited_to_the_scope() {
        let guard = ScopeGuard::default();
        let advertise = |uuid: &str| Command::StartAdvertising {
            server_id: ServerId::new("server-1"),
            options: AdvertisingOptions {
                service_uuid: uuid.into(),
                local_name: None,
                local_name_optional: true,
            },
        };
        assert_eq!(
            code(guard.authorize(&scope(), &owner(), advertise(OTHER))),
            ErrorCode::PermissionDenied
        );
        assert_eq!(
            guard.authorize(
                &scope(),
                &owner(),
                advertise("80FF87C3-8E84-4914-AEDC-0D6A3BA5534D")
            ),
            Ok(advertise(LAB))
        );
    }
}
