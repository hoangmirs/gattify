use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use parking_lot::Mutex;

use crate::{
    validate_server_definition, AdapterState, AdvertisementData, AdvertisingOptions,
    AdvertisingReport, Backend, BleError, BleResult, Capabilities, CharacteristicHandle,
    CharacteristicInstance, Command, ConnectionId, DeviceId, DiscoveredDevice, ErrorCode, Event,
    EventSink, LinkLimits, OperationContext, OwnerId, PeerId, PermissionOutcome, PermissionState,
    Reply, ResourceId, ResourceSnapshot, ScanId, ServerDefinition, ServerId, ServiceHandle,
    ServiceInstance, SubscriptionId, Support,
};

const DEFAULT_VALUE_LIMIT: u32 = 20;

struct Link {
    owner: OwnerId,
    /// The side and server at the other end. `None` for a device outside the air.
    remote: Option<(usize, ServerId)>,
    characteristics: HashMap<CharacteristicHandle, String>,
}

struct Subscription {
    owner: OwnerId,
    connection_id: ConnectionId,
    characteristic_key: String,
}

struct Server {
    owner: OwnerId,
    definition: ServerDefinition,
    values: HashMap<String, Vec<u8>>,
    /// The subscribed centrals, as (side, characteristic key).
    subscribers: HashSet<(usize, String)>,
}

struct Side {
    sink: EventSink,
    drop_frames: usize,
    cancelled: HashSet<String>,
    scans: HashMap<ScanId, OwnerId>,
    links: HashMap<ConnectionId, Link>,
    subscriptions: HashMap<SubscriptionId, Subscription>,
    servers: HashMap<ServerId, Server>,
    advertising: Option<(ServerId, AdvertisingOptions)>,
}

impl Side {
    fn new(sink: EventSink) -> Self {
        Self {
            sink,
            drop_frames: 0,
            cancelled: HashSet::new(),
            scans: HashMap::new(),
            links: HashMap::new(),
            subscriptions: HashMap::new(),
            servers: HashMap::new(),
            advertising: None,
        }
    }
}

struct Air {
    next_resource: u64,
    value_limit: u32,
    sides: Vec<Side>,
    /// Events raised while the air is locked, emitted after it is released.
    outbox: Vec<(usize, OwnerId, Event)>,
}

impl Air {
    fn new(sinks: Vec<EventSink>) -> Self {
        Self {
            next_resource: 0,
            value_limit: DEFAULT_VALUE_LIMIT,
            sides: sinks.into_iter().map(Side::new).collect(),
            outbox: Vec::new(),
        }
    }

    fn allocate(&mut self, prefix: &str) -> String {
        self.next_resource += 1;
        format!("{prefix}-{}", self.next_resource)
    }

    fn emit(&mut self, side: usize, owner: &OwnerId, event: Event) {
        self.outbox.push((side, owner.clone(), event));
    }

    fn take_frame_drop(&mut self, side: usize) -> bool {
        let drops = &mut self.sides[side].drop_frames;
        if *drops == 0 {
            return false;
        }
        *drops -= 1;
        true
    }

    fn link(&self, side: usize, connection_id: &ConnectionId, owner: &OwnerId) -> BleResult<&Link> {
        match self.sides[side].links.get(connection_id) {
            Some(link) if &link.owner == owner => Ok(link),
            _ => Err(BleError::invalid_handle(connection_id.clone())),
        }
    }

    fn server(&self, side: usize, server_id: &ServerId, owner: &OwnerId) -> BleResult<&Server> {
        match self.sides[side].servers.get(server_id) {
            Some(server) if &server.owner == owner => Ok(server),
            _ => Err(BleError::invalid_handle(server_id.clone())),
        }
    }

    /// The characteristic key behind `handle`, with the remote side and server.
    fn remote_characteristic(
        &self,
        side: usize,
        connection_id: &ConnectionId,
        handle: &CharacteristicHandle,
        owner: &OwnerId,
    ) -> BleResult<Option<(usize, ServerId, String)>> {
        let link = self.link(side, connection_id, owner)?;
        let Some((remote, server_id)) = &link.remote else {
            return Ok(None);
        };
        let key = link
            .characteristics
            .get(handle)
            .ok_or_else(|| BleError::invalid_handle(ResourceId::new(handle.as_str())))?;
        Ok(Some((*remote, server_id.clone(), key.clone())))
    }

    /// Ends one subscription and tells the server at the other end.
    fn end_subscription(&mut self, side: usize, subscription_id: &SubscriptionId) {
        let Some(subscription) = self.sides[side].subscriptions.remove(subscription_id) else {
            return;
        };
        let remote = self.sides[side]
            .links
            .get(&subscription.connection_id)
            .and_then(|link| link.remote.clone());
        if let Some((remote, server_id)) = remote {
            self.unsubscribe_remote(remote, &server_id, side, subscription.characteristic_key);
        }
    }

    fn unsubscribe_remote(
        &mut self,
        remote: usize,
        server_id: &ServerId,
        central_side: usize,
        characteristic_key: String,
    ) {
        let Some(server) = self.sides[remote].servers.get_mut(server_id) else {
            return;
        };
        if !server
            .subscribers
            .remove(&(central_side, characteristic_key.clone()))
        {
            return;
        }
        let owner = server.owner.clone();
        self.emit(
            remote,
            &owner,
            Event::SubscriptionChanged {
                server_id: server_id.clone(),
                peer_id: MockAir::central_id(central_side),
                characteristic_key,
                subscribed: false,
                max_value_length: None,
            },
        );
    }

    /// Removes a link, its subscriptions, and the matching subscribers remotely.
    fn drop_link(&mut self, side: usize, connection_id: &ConnectionId) -> Option<Link> {
        let subscriptions: Vec<_> = self.sides[side]
            .subscriptions
            .iter()
            .filter(|(_, subscription)| &subscription.connection_id == connection_id)
            .map(|(id, _)| id.clone())
            .collect();
        for subscription_id in subscriptions {
            self.end_subscription(side, &subscription_id);
        }
        self.sides[side].links.remove(connection_id)
    }

    /// Removes a server. Every central linked to it sees its link close.
    fn drop_server(&mut self, side: usize, server_id: &ServerId) {
        if self.sides[side]
            .advertising
            .as_ref()
            .is_some_and(|(advertised, _)| advertised == server_id)
        {
            self.sides[side].advertising = None;
        }
        self.sides[side].servers.remove(server_id);
        for central in 0..self.sides.len() {
            let closed: Vec<_> = self.sides[central]
                .links
                .iter()
                .filter(|(_, link)| link.remote.as_ref() == Some(&(side, server_id.clone())))
                .map(|(id, _)| id.clone())
                .collect();
            for connection_id in closed {
                if let Some(link) = self.drop_link(central, &connection_id) {
                    self.emit(
                        central,
                        &link.owner,
                        Event::ConnectionClosed { connection_id },
                    );
                }
            }
        }
    }

    fn limits(&self) -> LinkLimits {
        LinkLimits {
            write_with_response: Some(self.value_limit),
            write_without_response: Some(self.value_limit),
            notification: Some(self.value_limit),
            att_mtu: Some(self.value_limit + 3),
        }
    }

    // A single exhaustive dispatcher keeps mock behavior aligned with Command.
    #[allow(clippy::too_many_lines)]
    fn execute(
        &mut self,
        side: usize,
        context: OperationContext,
        command: Command,
    ) -> BleResult<Reply> {
        if self.sides[side]
            .cancelled
            .contains(context.operation_id.as_str())
        {
            return Err(BleError::new(
                ErrorCode::Cancelled,
                "operation was cancelled",
            ));
        }
        let owner = context.owner_id;

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
            Command::StartScan(options) => {
                let scan_id = ScanId::new(self.allocate("scan"));
                self.sides[side]
                    .scans
                    .insert(scan_id.clone(), owner.clone());
                let found: Vec<_> = (0..self.sides.len())
                    .filter(|other| *other != side)
                    .filter_map(|other| {
                        let (_, advertised) = self.sides[other].advertising.as_ref()?;
                        (options.service_uuids.is_empty()
                            || options.service_uuids.contains(&advertised.service_uuid))
                        .then(|| (other, advertised.clone()))
                    })
                    .collect();
                for (other, advertised) in found {
                    let device = DiscoveredDevice {
                        id: MockAir::device_id(other),
                        name: advertised.local_name.clone(),
                        rssi: Some(-40),
                        service_uuids: vec![advertised.service_uuid],
                        advertisement: Some(AdvertisementData {
                            local_name: advertised.local_name,
                            service_data: Vec::new(),
                            manufacturer_data: Vec::new(),
                            connectable: Some(true),
                        }),
                        observed_at_millis: 0,
                        scan_id: scan_id.clone(),
                    };
                    self.emit(side, &owner, Event::ScanResult { device });
                }
                Ok(Reply::ScanStarted { scan_id })
            }
            Command::StopScan { scan_id } => {
                match self.sides[side].scans.get(&scan_id) {
                    Some(scan_owner) if scan_owner == &owner => {}
                    _ => return Err(BleError::invalid_handle(scan_id)),
                }
                self.sides[side].scans.remove(&scan_id);
                Ok(Reply::Empty)
            }
            Command::Connect { device_id, .. } => {
                let remote = match MockAir::side_of(&device_id) {
                    Some(other) if other != side && other < self.sides.len() => {
                        let Some((server_id, _)) = &self.sides[other].advertising else {
                            return Err(BleError::new(
                                ErrorCode::Timeout,
                                "the mock device is not advertising",
                            ));
                        };
                        Some((other, server_id.clone()))
                    }
                    _ => None,
                };
                let connection_id = ConnectionId::new(self.allocate("connection"));
                self.sides[side].links.insert(
                    connection_id.clone(),
                    Link {
                        owner,
                        remote,
                        characteristics: HashMap::new(),
                    },
                );
                Ok(Reply::Connected {
                    connection_id,
                    limits: self.limits(),
                })
            }
            Command::Disconnect { connection_id } => {
                self.link(side, &connection_id, &owner)?;
                self.drop_link(side, &connection_id);
                Ok(Reply::Empty)
            }
            Command::DiscoverServices { connection_id } => {
                let remote = self.link(side, &connection_id, &owner)?.remote.clone();
                let Some((remote, server_id)) = remote else {
                    return Ok(Reply::Services(Vec::new()));
                };
                let Some(server) = self.sides[remote].servers.get(&server_id) else {
                    return Ok(Reply::Services(Vec::new()));
                };
                let mut handles = HashMap::new();
                let mut services = Vec::new();
                let mut next = 0;
                for (service_index, service) in server.definition.services.iter().enumerate() {
                    let mut characteristics = Vec::new();
                    for characteristic in &service.characteristics {
                        next += 1;
                        let handle = CharacteristicHandle::new(format!(
                            "{connection_id}/characteristic-{next}"
                        ));
                        handles.insert(
                            handle.clone(),
                            format!("{}/{}", service.instance_key, characteristic.instance_key),
                        );
                        characteristics.push(CharacteristicInstance {
                            handle,
                            uuid: characteristic.uuid.clone(),
                            properties: characteristic.properties.clone(),
                        });
                    }
                    services.push(ServiceInstance {
                        handle: ServiceHandle::new(format!(
                            "{connection_id}/service-{}",
                            service_index + 1
                        )),
                        uuid: service.uuid.clone(),
                        characteristics,
                    });
                }
                if let Some(link) = self.sides[side].links.get_mut(&connection_id) {
                    link.characteristics = handles;
                }
                Ok(Reply::Services(services))
            }
            Command::Read {
                connection_id,
                characteristic,
            } => {
                let value = match self.remote_characteristic(
                    side,
                    &connection_id,
                    &characteristic,
                    &owner,
                )? {
                    Some((remote, server_id, key)) => self.sides[remote]
                        .servers
                        .get(&server_id)
                        .and_then(|server| server.values.get(&key).cloned())
                        .unwrap_or_default(),
                    None => Vec::new(),
                };
                Ok(Reply::Bytes {
                    value_base64: BASE64.encode(value),
                })
            }
            Command::Write {
                connection_id,
                characteristic,
                value_base64,
                ..
            } => {
                let target =
                    self.remote_characteristic(side, &connection_id, &characteristic, &owner)?;
                decode(&value_base64)?;
                let Some((remote, server_id, characteristic_key)) = target else {
                    return Ok(Reply::Empty);
                };
                if self.take_frame_drop(side) {
                    return Ok(Reply::Empty);
                }
                if let Some(server) = self.sides[remote].servers.get(&server_id) {
                    let server_owner = server.owner.clone();
                    self.emit(
                        remote,
                        &server_owner,
                        Event::ServerWrite {
                            server_id,
                            peer_id: Some(MockAir::central_id(side)),
                            characteristic_key,
                            value_base64,
                        },
                    );
                }
                Ok(Reply::Empty)
            }
            Command::Subscribe {
                connection_id,
                characteristic,
            } => {
                let target =
                    self.remote_characteristic(side, &connection_id, &characteristic, &owner)?;
                let subscription_id = SubscriptionId::new(self.allocate("subscription"));
                let characteristic_key = target
                    .as_ref()
                    .map_or_else(String::new, |(_, _, key)| key.clone());
                if let Some((remote, server_id, key)) = target {
                    let value_limit = self.value_limit;
                    if let Some(server) = self.sides[remote].servers.get_mut(&server_id) {
                        server.subscribers.insert((side, key.clone()));
                        let server_owner = server.owner.clone();
                        self.emit(
                            remote,
                            &server_owner,
                            Event::SubscriptionChanged {
                                server_id,
                                peer_id: MockAir::central_id(side),
                                characteristic_key: key,
                                subscribed: true,
                                max_value_length: Some(value_limit),
                            },
                        );
                    }
                }
                self.sides[side].subscriptions.insert(
                    subscription_id.clone(),
                    Subscription {
                        owner,
                        connection_id,
                        characteristic_key,
                    },
                );
                Ok(Reply::SubscriptionStarted { subscription_id })
            }
            Command::Unsubscribe { subscription_id } => {
                let is_callers = self.sides[side]
                    .subscriptions
                    .get(&subscription_id)
                    .is_some_and(|subscription| subscription.owner == owner);
                if !is_callers {
                    return Err(BleError::invalid_handle(subscription_id));
                }
                self.end_subscription(side, &subscription_id);
                Ok(Reply::Empty)
            }
            Command::CreateServer(definition) => {
                validate_server_definition(&definition)?;
                let mut values = HashMap::new();
                for service in &definition.services {
                    for characteristic in &service.characteristics {
                        let value = characteristic
                            .initial_value_base64
                            .as_deref()
                            .map(decode)
                            .transpose()?
                            .unwrap_or_default();
                        values.insert(
                            format!("{}/{}", service.instance_key, characteristic.instance_key),
                            value,
                        );
                    }
                }
                let server_id = ServerId::new(self.allocate("server"));
                self.sides[side].servers.insert(
                    server_id.clone(),
                    Server {
                        owner,
                        definition,
                        values,
                        subscribers: HashSet::new(),
                    },
                );
                Ok(Reply::ServerCreated { server_id })
            }
            Command::CloseServer { server_id } => {
                self.server(side, &server_id, &owner)?;
                self.drop_server(side, &server_id);
                Ok(Reply::Empty)
            }
            Command::StartAdvertising { server_id, options } => {
                self.server(side, &server_id, &owner)?;
                crate::validate_uuid(&options.service_uuid)?;
                let report = AdvertisingReport {
                    local_name_included: options.local_name.is_some(),
                    local_name_truncated: false,
                };
                self.sides[side].advertising = Some((server_id, options));
                Ok(Reply::AdvertisingStarted(report))
            }
            Command::StopAdvertising { server_id } => {
                self.server(side, &server_id, &owner)?;
                if self.sides[side]
                    .advertising
                    .as_ref()
                    .is_some_and(|(advertised, _)| advertised == &server_id)
                {
                    self.sides[side].advertising = None;
                }
                Ok(Reply::Empty)
            }
            Command::SetValue {
                server_id,
                characteristic_key,
                value_base64,
            } => {
                self.server(side, &server_id, &owner)?;
                let value = decode(&value_base64)?;
                if let Some(server) = self.sides[side].servers.get_mut(&server_id) {
                    server.values.insert(characteristic_key, value);
                }
                Ok(Reply::Empty)
            }
            Command::Notify {
                server_id,
                peer_id,
                characteristic_key,
                value_base64,
            } => {
                let server = self.server(side, &server_id, &owner)?;
                decode(&value_base64)?;
                let central = MockAir::side_of_central(&peer_id)
                    .filter(|central| {
                        server
                            .subscribers
                            .contains(&(*central, characteristic_key.clone()))
                    })
                    .ok_or_else(|| BleError::invalid_handle(ResourceId::new(peer_id.as_str())))?;
                if self.take_frame_drop(side) {
                    return Ok(Reply::Empty);
                }
                let target = self.sides[central]
                    .subscriptions
                    .iter()
                    .find(|(_, subscription)| {
                        subscription.characteristic_key == characteristic_key
                            && self.sides[central]
                                .links
                                .get(&subscription.connection_id)
                                .is_some_and(|link| {
                                    link.remote.as_ref() == Some(&(side, server_id.clone()))
                                })
                    })
                    .map(|(id, subscription)| (id.clone(), subscription.owner.clone()));
                if let Some((subscription_id, subscriber)) = target {
                    self.emit(
                        central,
                        &subscriber,
                        Event::CharacteristicValue {
                            subscription_id,
                            value_base64,
                        },
                    );
                }
                Ok(Reply::Empty)
            }
            Command::Cancel { operation_id } => {
                self.sides[side].cancelled.insert(operation_id.0);
                Ok(Reply::Empty)
            }
            Command::CloseOwner => {
                self.sides[side]
                    .scans
                    .retain(|_, scan_owner| scan_owner != &owner);
                let links: Vec<_> = self.sides[side]
                    .links
                    .iter()
                    .filter(|(_, link)| link.owner == owner)
                    .map(|(id, _)| id.clone())
                    .collect();
                for connection_id in links {
                    self.drop_link(side, &connection_id);
                }
                let subscriptions: Vec<_> = self.sides[side]
                    .subscriptions
                    .iter()
                    .filter(|(_, subscription)| subscription.owner == owner)
                    .map(|(id, _)| id.clone())
                    .collect();
                for subscription_id in subscriptions {
                    self.end_subscription(side, &subscription_id);
                }
                let servers: Vec<_> = self.sides[side]
                    .servers
                    .iter()
                    .filter(|(_, server)| server.owner == owner)
                    .map(|(id, _)| id.clone())
                    .collect();
                for server_id in servers {
                    self.drop_server(side, &server_id);
                }
                Ok(Reply::Empty)
            }
            Command::DebugResources => {
                let state = &self.sides[side];
                Ok(Reply::Resources(ResourceSnapshot {
                    scans: state
                        .scans
                        .values()
                        .filter(|value| **value == owner)
                        .count(),
                    connections: state
                        .links
                        .values()
                        .filter(|link| link.owner == owner)
                        .count(),
                    subscriptions: state
                        .subscriptions
                        .values()
                        .filter(|subscription| subscription.owner == owner)
                        .count(),
                    servers: state
                        .servers
                        .values()
                        .filter(|server| server.owner == owner)
                        .count(),
                }))
            }
        }
    }
}

fn decode(value_base64: &str) -> BleResult<Vec<u8>> {
    BASE64
        .decode(value_base64)
        .map_err(|_| BleError::new(ErrorCode::InvalidArgument, "invalid base64 payload"))
}

/// Emits the events that accumulated while the air was locked.
fn flush(air: &Mutex<Air>) {
    let (events, sinks) = {
        let mut air = air.lock();
        let sinks: Vec<_> = air.sides.iter().map(|side| side.sink.clone()).collect();
        (std::mem::take(&mut air.outbox), sinks)
    };
    for (side, owner, event) in events {
        (sinks[side])(owner, event);
    }
}

/// Deterministic backend for unit tests and example development.
///
/// It is intentionally never selected by the production plugin constructor.
/// A backend made by [`MockAir::link`] talks to another mock backend.
pub struct MockBackend {
    air: Arc<Mutex<Air>>,
    side: usize,
}

impl MockBackend {
    /// Creates a mock that raises its events through `sink`.
    #[must_use]
    pub fn new(sink: EventSink) -> Self {
        Self {
            air: Arc::new(Mutex::new(Air::new(vec![sink]))),
            side: 0,
        }
    }
}

impl Default for MockBackend {
    fn default() -> Self {
        Self::new(Arc::new(|_, _| {}))
    }
}

#[async_trait]
impl Backend for MockBackend {
    async fn execute(&self, context: OperationContext, command: Command) -> BleResult<Reply> {
        let reply = self.air.lock().execute(self.side, context, command);
        flush(&self.air);
        reply
    }
}

/// Links two mock backends over a simulated radio.
///
/// Side 0 is the first backend of [`MockAir::link`] and side 1 the second.
/// A write or a notification on one side arrives as an event on the other.
pub struct MockAir {
    air: Arc<Mutex<Air>>,
}

impl MockAir {
    /// Links two sides, each with the sink that receives its events.
    #[must_use]
    pub fn link(first: EventSink, second: EventSink) -> (Self, MockBackend, MockBackend) {
        let air = Arc::new(Mutex::new(Air::new(vec![first, second])));
        (
            Self { air: air.clone() },
            MockBackend {
                air: air.clone(),
                side: 0,
            },
            MockBackend { air, side: 1 },
        )
    }

    /// The device ID under which the other side sees `side`.
    #[must_use]
    pub fn device_id(side: usize) -> DeviceId {
        DeviceId::new(format!("mock-device-{side}"))
    }

    /// The central ID under which a server on the other side sees `side`.
    #[must_use]
    pub fn central_id(side: usize) -> PeerId {
        PeerId::new(format!("mock-central-{side}"))
    }

    fn side_of(device_id: &DeviceId) -> Option<usize> {
        device_id
            .as_str()
            .strip_prefix("mock-device-")?
            .parse()
            .ok()
    }

    fn side_of_central(peer_id: &PeerId) -> Option<usize> {
        peer_id.as_str().strip_prefix("mock-central-")?.parse().ok()
    }

    /// Drops the next write or notification that `side` sends. The command
    /// still succeeds, as a lost radio frame would.
    pub fn drop_next_frame(&self, side: usize) {
        self.air.lock().sides[side].drop_frames += 1;
    }

    /// Sets the value length that connections and subscriptions report.
    pub fn set_value_limit(&self, value_limit: u32) {
        self.air.lock().value_limit = value_limit;
    }

    /// Closes every link between the sides, as when the phones move apart.
    pub fn disconnect(&self) {
        {
            let mut air = self.air.lock();
            for side in 0..air.sides.len() {
                let linked: Vec<_> = air.sides[side]
                    .links
                    .iter()
                    .filter(|(_, link)| link.remote.is_some())
                    .map(|(id, _)| id.clone())
                    .collect();
                for connection_id in linked {
                    if let Some(link) = air.drop_link(side, &connection_id) {
                        air.emit(side, &link.owner, Event::ConnectionClosed { connection_id });
                    }
                }
            }
        }
        flush(&self.air);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CharacteristicProperties, ConnectOptions, LocalCharacteristic, LocalService, Manager,
        ScanOptions, WriteType,
    };

    type Recorded = Arc<Mutex<Vec<(OwnerId, Event)>>>;

    const SERVICE: &str = "80ff87c3-8e84-4914-aedc-0d6a3ba5534d";

    fn recorder() -> (EventSink, Recorded) {
        let recorded: Recorded = Arc::default();
        let sink_recorded = recorded.clone();
        let sink: EventSink =
            Arc::new(move |owner, event| sink_recorded.lock().push((owner, event)));
        (sink, recorded)
    }

    fn run(backend: &MockBackend, owner: &str, command: Command) -> BleResult<Reply> {
        futures_lite::future::block_on(backend.execute(
            OperationContext {
                operation_id: crate::OperationId::new("op"),
                owner_id: OwnerId::new(owner),
                deadline_millis: None,
            },
            command,
        ))
    }

    fn definition() -> ServerDefinition {
        let characteristic = |key: &str, uuid: &str, properties| LocalCharacteristic {
            instance_key: key.into(),
            uuid: uuid.into(),
            properties,
            initial_value_base64: Some("AQ==".into()),
            max_value_length: 512,
        };
        ServerDefinition {
            services: vec![LocalService {
                instance_key: "peer".into(),
                uuid: SERVICE.into(),
                primary: true,
                characteristics: vec![
                    characteristic(
                        "rx",
                        "b1e10f10-6a2c-4a62-8e9e-2c938fa30102",
                        CharacteristicProperties {
                            write: true,
                            ..CharacteristicProperties::default()
                        },
                    ),
                    characteristic(
                        "tx",
                        "b1e10f10-6a2c-4a62-8e9e-2c938fa30103",
                        CharacteristicProperties {
                            notify: true,
                            ..CharacteristicProperties::default()
                        },
                    ),
                ],
            }],
        }
    }

    struct Linked {
        air: MockAir,
        host: MockBackend,
        joiner: MockBackend,
        host_events: Recorded,
        joiner_events: Recorded,
        server_id: ServerId,
        connection_id: ConnectionId,
        rx: CharacteristicHandle,
        tx: CharacteristicHandle,
    }

    /// Side 0 hosts and advertises a server. Side 1 connects and discovers it.
    fn linked() -> Linked {
        let (host_sink, host_events) = recorder();
        let (joiner_sink, joiner_events) = recorder();
        let (air, host, joiner) = MockAir::link(host_sink, joiner_sink);
        let Reply::ServerCreated { server_id } =
            run(&host, "host", Command::CreateServer(definition())).unwrap()
        else {
            panic!("expected a server");
        };
        run(
            &host,
            "host",
            Command::StartAdvertising {
                server_id: server_id.clone(),
                options: AdvertisingOptions {
                    service_uuid: SERVICE.into(),
                    local_name: Some("Host".into()),
                    local_name_optional: true,
                },
            },
        )
        .unwrap();
        let Reply::Connected { connection_id, .. } = run(
            &joiner,
            "joiner",
            Command::Connect {
                device_id: MockAir::device_id(0),
                options: ConnectOptions { timeout_ms: None },
            },
        )
        .unwrap() else {
            panic!("expected a connection");
        };
        let Reply::Services(services) = run(
            &joiner,
            "joiner",
            Command::DiscoverServices {
                connection_id: connection_id.clone(),
            },
        )
        .unwrap() else {
            panic!("expected services");
        };
        let handle = |uuid: &str| {
            services[0]
                .characteristics
                .iter()
                .find(|characteristic| characteristic.uuid == uuid)
                .unwrap()
                .handle
                .clone()
        };
        Linked {
            rx: handle("b1e10f10-6a2c-4a62-8e9e-2c938fa30102"),
            tx: handle("b1e10f10-6a2c-4a62-8e9e-2c938fa30103"),
            air,
            host,
            joiner,
            host_events,
            joiner_events,
            server_id,
            connection_id,
        }
    }

    fn write(linked: &Linked, value_base64: &str) {
        run(
            &linked.joiner,
            "joiner",
            Command::Write {
                connection_id: linked.connection_id.clone(),
                characteristic: linked.rx.clone(),
                value_base64: value_base64.into(),
                write_type: WriteType::WithResponse,
            },
        )
        .unwrap();
    }

    fn subscribe(linked: &Linked) -> SubscriptionId {
        let Reply::SubscriptionStarted { subscription_id } = run(
            &linked.joiner,
            "joiner",
            Command::Subscribe {
                connection_id: linked.connection_id.clone(),
                characteristic: linked.tx.clone(),
            },
        )
        .unwrap() else {
            panic!("expected a subscription");
        };
        subscription_id
    }

    #[test]
    fn a_scan_finds_the_advertising_side() {
        let linked = linked();
        run(
            &linked.joiner,
            "joiner",
            Command::StartScan(ScanOptions {
                service_uuids: vec![SERVICE.into()],
                timeout_ms: None,
            }),
        )
        .unwrap();
        let events = linked.joiner_events.lock();
        let Some((owner, Event::ScanResult { device })) = events.last() else {
            panic!("expected a scan result");
        };
        assert_eq!(owner.as_str(), "joiner");
        assert_eq!(device.id, MockAir::device_id(0));
        assert_eq!(device.name.as_deref(), Some("Host"));
    }

    #[test]
    fn a_read_returns_the_stored_value() {
        let linked = linked();
        let reply = run(
            &linked.joiner,
            "joiner",
            Command::Read {
                connection_id: linked.connection_id.clone(),
                characteristic: linked.rx.clone(),
            },
        );
        assert_eq!(
            reply,
            Ok(Reply::Bytes {
                value_base64: "AQ==".into()
            })
        );
    }

    #[test]
    fn a_write_arrives_as_a_server_write() {
        let linked = linked();
        write(&linked, "AAE=");
        assert_eq!(
            linked.host_events.lock().last(),
            Some(&(
                OwnerId::new("host"),
                Event::ServerWrite {
                    server_id: linked.server_id.clone(),
                    peer_id: Some(MockAir::central_id(1)),
                    characteristic_key: "peer/rx".into(),
                    value_base64: "AAE=".into(),
                }
            ))
        );
    }

    #[test]
    fn a_notification_reaches_the_subscriber() {
        let linked = linked();
        linked.air.set_value_limit(182);
        let subscription_id = subscribe(&linked);
        assert_eq!(
            linked.host_events.lock().last(),
            Some(&(
                OwnerId::new("host"),
                Event::SubscriptionChanged {
                    server_id: linked.server_id.clone(),
                    peer_id: MockAir::central_id(1),
                    characteristic_key: "peer/tx".into(),
                    subscribed: true,
                    max_value_length: Some(182),
                }
            ))
        );
        run(
            &linked.host,
            "host",
            Command::Notify {
                server_id: linked.server_id.clone(),
                peer_id: MockAir::central_id(1),
                characteristic_key: "peer/tx".into(),
                value_base64: "Ag==".into(),
            },
        )
        .unwrap();
        assert_eq!(
            linked.joiner_events.lock().last(),
            Some(&(
                OwnerId::new("joiner"),
                Event::CharacteristicValue {
                    subscription_id,
                    value_base64: "Ag==".into(),
                }
            ))
        );
    }

    #[test]
    fn a_notification_to_a_central_without_a_subscription_is_rejected() {
        let linked = linked();
        let error = run(
            &linked.host,
            "host",
            Command::Notify {
                server_id: linked.server_id.clone(),
                peer_id: MockAir::central_id(1),
                characteristic_key: "peer/tx".into(),
                value_base64: "Ag==".into(),
            },
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidHandle);
    }

    #[test]
    fn a_dropped_frame_never_arrives() {
        let linked = linked();
        linked.air.drop_next_frame(1);
        write(&linked, "AA==");
        assert!(!linked
            .host_events
            .lock()
            .iter()
            .any(|(_, event)| matches!(event, Event::ServerWrite { .. })));
        write(&linked, "AQ==");
        assert!(matches!(
            linked.host_events.lock().last(),
            Some((_, Event::ServerWrite { value_base64, .. })) if value_base64 == "AQ=="
        ));
    }

    #[test]
    fn a_lost_link_reaches_both_sides() {
        let linked = linked();
        subscribe(&linked);
        linked.air.disconnect();
        assert_eq!(
            linked.joiner_events.lock().last(),
            Some(&(
                OwnerId::new("joiner"),
                Event::ConnectionClosed {
                    connection_id: linked.connection_id.clone()
                }
            ))
        );
        assert!(matches!(
            linked.host_events.lock().last(),
            Some((
                _,
                Event::SubscriptionChanged {
                    subscribed: false,
                    max_value_length: None,
                    ..
                }
            ))
        ));
    }

    #[test]
    fn a_disconnect_ends_the_remote_subscription_silently_for_the_caller() {
        let linked = linked();
        subscribe(&linked);
        let joiner_events_before = linked.joiner_events.lock().len();
        run(
            &linked.joiner,
            "joiner",
            Command::Disconnect {
                connection_id: linked.connection_id.clone(),
            },
        )
        .unwrap();
        assert_eq!(linked.joiner_events.lock().len(), joiner_events_before);
        assert!(matches!(
            linked.host_events.lock().last(),
            Some((
                _,
                Event::SubscriptionChanged {
                    subscribed: false,
                    ..
                }
            ))
        ));
    }

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
