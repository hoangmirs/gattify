use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    future::IntoFuture,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use tokio::task::AbortHandle;
use windows::{
    core::IInspectable,
    Devices::Bluetooth::{
        BluetoothError,
        GenericAttributeProfile::{
            GattCharacteristicProperties, GattCommunicationStatus, GattLocalCharacteristic,
            GattLocalCharacteristicParameters, GattLocalCharacteristicResult, GattProtectionLevel,
            GattReadRequest, GattReadRequestedEventArgs, GattServiceProvider,
            GattServiceProviderAdvertisementStatus,
            GattServiceProviderAdvertisementStatusChangedEventArgs,
            GattServiceProviderAdvertisingParameters, GattServiceProviderResult,
            GattSubscribedClient, GattWriteOption, GattWriteRequest, GattWriteRequestedEventArgs,
        },
    },
    Foundation::Deferral,
};
use windows_future::IAsyncOperation;

use super::{
    advertise::{
        advertising_report, cut_local_name, LocalNameCut, ProviderStatus, Publication, PublishStep,
        Published, Publisher, LOCAL_NAME_BUDGET,
    },
    convert::{att_error, buffer, bytes, guid, handler, items, session_device, winrt_error},
    engine::{busy, cancelled, decode, disconnected, invalid_argument, timeout, Cleanup, Engine},
    gatt::{
        att, notification_length, properties_to_bits, read_answer, subscriber_changes, write_error,
        SubscriberChange,
    },
    queue::ProcedureQueue,
    status::{bluetooth_error, communication_error},
};
use crate::{
    normalize_uuid, validate_server_definition, AdvertisingOptions, BleError, BleResult,
    CharacteristicProperties, ErrorCode, Event, OwnerId, PeerId, Reply, ServerDefinition, ServerId,
};

/// How long past its deadline a notification may stay unanswered before the next one goes.
const NOTIFICATION_GRACE: Duration = Duration::from_secs(2);
/// The deadline of a notification without one.
const NOTIFICATION_DEADLINE: Duration = Duration::from_secs(5);
/// How often the status of a provider is read while a call waits for it.
const PUBLICATION_CHECK: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ServerState {
    Registering,
    Ready,
    /// The radio turned off. Only `closeServer` and `stopAdvertising` still work.
    Lost,
}

pub(super) struct Server {
    pub(super) owner: OwnerId,
    state: ServerState,
    create_key: Option<u64>,
    definition: ServerDefinition,
    service_uuids: BTreeSet<String>,
    /// One provider per service of the definition, in its order.
    services: Vec<LocalService>,
    characteristics: BTreeMap<String, LocalAttribute>,
    notifications: ProcedureQueue<PendingNotification>,
}

impl Server {
    pub(super) fn is_registering(&self) -> bool {
        self.state == ServerState::Registering
    }
}

struct LocalService {
    provider: GattServiceProvider,
    status_token: Option<i64>,
    publisher: Publisher,
    /// Reads the status again while a call waits for it.
    watchdog: Option<AbortHandle>,
}

struct LocalAttribute {
    characteristic: Option<GattLocalCharacteristic>,
    tokens: [Option<i64>; 3],
    properties: CharacteristicProperties,
    max_value_length: u32,
    /// The value reads return. Writes and notifications leave it unchanged.
    value: Vec<u8>,
    /// By the Windows device ID of each subscribed central.
    subscribers: BTreeMap<String, Subscriber>,
}

struct Subscriber {
    central: PeerId,
    client: GattSubscribedClient,
    length: u32,
    size_token: Option<i64>,
}

/// The one advertisement of the process.
pub(super) struct AdvertisementRecord {
    server_id: ServerId,
    service_index: usize,
    generation: u64,
    /// The pending `startAdvertising`, until the provider reports its status.
    starting: Option<u64>,
    cut: LocalNameCut,
    name_optional: bool,
    /// Windows 10 1809 and earlier cannot carry the name as service data.
    service_data_set: bool,
}

struct PendingNotification {
    token: u64,
    key: u64,
    central: PeerId,
    characteristic_key: String,
    value: Vec<u8>,
    deadline_at: Instant,
    watchdog: Option<AbortHandle>,
}

/// A write request that arrived, with what answering it needs.
struct WriteArrival {
    deferral: Deferral,
    request: IAsyncOperation<GattWriteRequest>,
    central: Option<String>,
}

/// A write request ready to be answered, in the order the writes arrived.
pub(super) struct ReadyWrite {
    server_id: ServerId,
    characteristic_key: String,
    central: Option<String>,
    deferral: Deferral,
    request: Option<GattWriteRequest>,
}

// Commands

impl Engine {
    pub(super) fn create_server(&mut self, key: u64, mut definition: ServerDefinition) {
        if let Err(error) = validate_server_definition(&definition) {
            return self.reject(key, error);
        }
        for service in &mut definition.services {
            service.uuid = normalize_uuid(&service.uuid).unwrap_or_default();
            for characteristic in &mut service.characteristics {
                characteristic.uuid = normalize_uuid(&characteristic.uuid).unwrap_or_default();
            }
        }
        if let Err(error) = self.check_free(&definition) {
            return self.reject(key, error);
        }
        self.when_adapter(key, true, move |engine, key| {
            engine.register(key, definition);
        });
    }

    pub(super) fn close_server(&mut self, key: u64, server_id: &ServerId) {
        if let Err(error) = self.owned_server(key, server_id, true) {
            return self.reject(key, error);
        }
        self.discard_server(server_id, &cancelled());
        self.resolve(key, Reply::Empty);
    }

    pub(super) fn start_advertising(
        &mut self,
        key: u64,
        server_id: &ServerId,
        options: &AdvertisingOptions,
    ) {
        let checked = self.owned_server(key, server_id, false).and_then(|()| {
            let uuid = normalize_uuid(&options.service_uuid).ok_or_else(|| {
                invalid_argument(format!("{} is not a UUID", options.service_uuid))
            })?;
            let index = self.servers.get(server_id).and_then(|server| {
                server
                    .definition
                    .services
                    .iter()
                    .position(|service| service.uuid == uuid)
            });
            let index = index
                .ok_or_else(|| invalid_argument(format!("{server_id} has no service {uuid}")))?;
            let cut = cut_local_name(options.local_name.as_deref(), LOCAL_NAME_BUDGET);
            advertising_report(&cut, true, options.local_name_optional)?;
            self.check_advertiser(server_id)?;
            Ok((index, cut))
        });
        let (index, cut) = match checked {
            Ok(checked) => checked,
            Err(error) => return self.reject(key, error),
        };
        let server_id = server_id.clone();
        let name_optional = options.local_name_optional;
        self.when_adapter(key, true, move |engine, key| {
            engine.advertise(key, &server_id, index, cut, name_optional);
        });
    }

    pub(super) fn stop_advertising(&mut self, key: u64, server_id: &ServerId) {
        if let Err(error) = self.owned_server(key, server_id, true) {
            return self.reject(key, error);
        }
        if self
            .advertisement
            .as_ref()
            .is_some_and(|advertisement| &advertisement.server_id == server_id)
        {
            self.stop_advertisement(&cancelled());
        }
        self.resolve(key, Reply::Empty);
    }

    pub(super) fn set_value(
        &mut self,
        key: u64,
        server_id: &ServerId,
        characteristic_key: &str,
        value_base64: &str,
    ) {
        let checked = self
            .owned_server(key, server_id, false)
            .and_then(|()| decode(value_base64));
        let value = match checked {
            Ok(value) => value,
            Err(error) => return self.reject(key, error),
        };
        let Some(attribute) = self
            .servers
            .get_mut(server_id)
            .and_then(|server| server.characteristics.get_mut(characteristic_key))
        else {
            return self.reject(
                key,
                invalid_argument(format!(
                    "{server_id} has no characteristic {characteristic_key}"
                )),
            );
        };
        if value.len() > attribute.max_value_length as usize {
            let error = BleError::new(
                ErrorCode::PayloadTooLarge,
                format!(
                    "the value is longer than maxValueLength {}",
                    attribute.max_value_length
                ),
            );
            return self.reject(key, error);
        }
        attribute.value = value;
        self.resolve(key, Reply::Empty);
    }

    pub(super) fn notify(
        &mut self,
        key: u64,
        server_id: &ServerId,
        peer_id: PeerId,
        characteristic_key: String,
        value_base64: &str,
    ) {
        let checked = self.owned_server(key, server_id, false).and_then(|()| {
            let attribute = self
                .servers
                .get(server_id)
                .and_then(|server| server.characteristics.get(&characteristic_key))
                .ok_or_else(|| {
                    invalid_argument(format!(
                        "{server_id} has no characteristic {characteristic_key}"
                    ))
                })?;
            let length = attribute
                .subscribers
                .values()
                .find(|subscriber| subscriber.central == peer_id)
                .map(|subscriber| subscriber.length)
                .ok_or_else(|| {
                    BleError::new(
                        ErrorCode::InvalidHandle,
                        format!("{peer_id} has no subscription to {characteristic_key}"),
                    )
                })?;
            let value = decode(value_base64)?;
            if value.len() > length as usize {
                return Err(BleError::new(
                    ErrorCode::PayloadTooLarge,
                    format!("a notification to {peer_id} carries at most {length} bytes"),
                ));
            }
            if let Some(error) = self.radio_error() {
                return Err(error);
            }
            Ok(value)
        });
        let value = match checked {
            Ok(value) => value,
            Err(error) => return self.reject(key, error),
        };
        let token = self.token();
        let deadline_at = self
            .deadline_at(key)
            .unwrap_or_else(|| Instant::now() + NOTIFICATION_DEADLINE);
        self.set_cleanup(key, Cleanup::Notify(server_id.clone(), token));
        if let Some(server) = self.servers.get_mut(server_id) {
            server.notifications.push(PendingNotification {
                token,
                key,
                central: peer_id,
                characteristic_key,
                value,
                deadline_at,
                watchdog: None,
            });
        }
        self.send_notifications(server_id);
    }

    /// A server of the caller. A lost server only accepts cleanup.
    fn owned_server(&self, key: u64, server_id: &ServerId, allow_lost: bool) -> BleResult<()> {
        let owner = self.owner(key);
        let server = self
            .servers
            .get(server_id)
            .filter(|server| Some(&server.owner) == owner.as_ref())
            .filter(|server| {
                server.state == ServerState::Ready
                    || (allow_lost && server.state == ServerState::Lost)
            });
        server
            .map(|_| ())
            .ok_or_else(|| BleError::invalid_handle(server_id.clone()))
    }

    /// Rejects with `busy` when a live server registered a service UUID of `definition`.
    fn check_free(&self, definition: &ServerDefinition) -> BleResult<()> {
        let taken = self
            .servers
            .values()
            .filter(|server| server.state != ServerState::Lost)
            .flat_map(|server| server.service_uuids.iter())
            .find(|uuid| {
                definition
                    .services
                    .iter()
                    .any(|service| &service.uuid == *uuid)
            });
        match taken {
            Some(uuid) => Err(busy(format!(
                "another server registered the service {uuid}"
            ))),
            None => Ok(()),
        }
    }

    /// One advertisement exists per process, and one start at a time.
    fn check_advertiser(&self, server_id: &ServerId) -> BleResult<()> {
        match &self.advertisement {
            Some(current) if &current.server_id != server_id => {
                Err(busy("another server advertises"))
            }
            Some(current) if current.starting.is_some() => {
                Err(busy("the advertisement of this server is starting"))
            }
            _ => Ok(()),
        }
    }

    fn peripheral_error(&self) -> Option<BleError> {
        match self.adapter_facts() {
            Some(facts) if facts.peripheral => None,
            _ => Some(BleError::unsupported(
                "this adapter cannot act as a peripheral",
            )),
        }
    }
}

// Registration

impl Engine {
    fn register(&mut self, key: u64, definition: ServerDefinition) {
        if let Some(error) = self.peripheral_error() {
            return self.reject(key, error);
        }
        if let Err(error) = self.check_free(&definition) {
            return self.reject(key, error);
        }
        let Some(owner) = self.owner(key) else {
            return;
        };
        let mut characteristics = BTreeMap::new();
        for service in &definition.services {
            for characteristic in &service.characteristics {
                let value = characteristic
                    .initial_value_base64
                    .as_deref()
                    .map(decode)
                    .transpose()
                    .unwrap_or_default()
                    .unwrap_or_default();
                characteristics.insert(
                    format!("{}/{}", service.instance_key, characteristic.instance_key),
                    LocalAttribute {
                        characteristic: None,
                        tokens: [None, None, None],
                        properties: characteristic.properties.clone(),
                        max_value_length: characteristic.max_value_length,
                        value,
                        subscribers: BTreeMap::new(),
                    },
                );
            }
        }
        let server_id = ServerId::new(self.ids.next("server"));
        self.servers.insert(
            server_id.clone(),
            Server {
                owner,
                state: ServerState::Registering,
                create_key: Some(key),
                service_uuids: definition
                    .services
                    .iter()
                    .map(|service| service.uuid.clone())
                    .collect(),
                definition,
                services: Vec::new(),
                characteristics,
                notifications: ProcedureQueue::default(),
            },
        );
        self.set_cleanup(key, Cleanup::CreateServer(server_id.clone()));
        self.add_service(server_id, key, 0);
    }

    fn registering(&mut self, server_id: &ServerId) -> Option<&mut Server> {
        self.servers
            .get_mut(server_id)
            .filter(|server| server.state == ServerState::Registering)
    }

    /// Creates one service provider per service, one after another.
    fn add_service(&mut self, server_id: ServerId, key: u64, index: usize) {
        let Some(server) = self.registering(&server_id) else {
            return;
        };
        let Some(uuid) = server
            .definition
            .services
            .get(index)
            .map(|service| service.uuid.clone())
        else {
            server.state = ServerState::Ready;
            server.create_key = None;
            self.set_cleanup(key, Cleanup::None);
            return self.resolve(key, Reply::ServerCreated { server_id });
        };
        let created = guid(&uuid)
            .ok_or_else(|| invalid_argument(format!("{uuid} is not a UUID")))
            .and_then(|uuid| {
                GattServiceProvider::CreateAsync(uuid)
                    .map_err(|error| winrt_error(&error, "creating the service"))
            });
        match created {
            Ok(operation) => self.spawn(operation.into_future(), move |engine, result| {
                engine.service_created(server_id, key, index, result);
            }),
            Err(error) => self.fail_registration(&server_id, key, error),
        }
    }

    fn service_created(
        &mut self,
        server_id: ServerId,
        key: u64,
        index: usize,
        result: windows::core::Result<GattServiceProviderResult>,
    ) {
        let action = "creating the service";
        let provider = result
            .map_err(|error| winrt_error(&error, action))
            .and_then(|result| {
                let error = result
                    .Error()
                    .map_err(|error| winrt_error(&error, action))?;
                if error != BluetoothError::Success {
                    return Err(bluetooth_error(error.0, action));
                }
                result
                    .ServiceProvider()
                    .map_err(|error| winrt_error(&error, action))
            });
        if self.registering(&server_id).is_none() {
            return;
        }
        let provider = match provider {
            Ok(provider) => provider,
            Err(error) => return self.fail_registration(&server_id, key, error),
        };
        let poster = self.poster.clone();
        let id = server_id.clone();
        let status_token = provider
            .AdvertisementStatusChanged(&handler::<
                GattServiceProvider,
                GattServiceProviderAdvertisementStatusChangedEventArgs,
            >(move |args| {
                let Some(args) = args else {
                    return;
                };
                let reported = args.Status().map_or(ProviderStatus::Other, provider_status);
                let error = args.Error().unwrap_or_default();
                let id = id.clone();
                poster.post(move |engine| engine.read_provider(&id, index, false, reported, error));
            }))
            .ok();
        if let Some(server) = self.registering(&server_id) {
            server.services.push(LocalService {
                provider,
                status_token,
                publisher: Publisher::default(),
                watchdog: None,
            });
        }
        self.add_characteristic(server_id, key, index, 0);
    }

    fn add_characteristic(
        &mut self,
        server_id: ServerId,
        key: u64,
        service_index: usize,
        index: usize,
    ) {
        let Some(server) = self.registering(&server_id) else {
            return;
        };
        let service = &server.definition.services[service_index];
        let Some(characteristic) = service.characteristics.get(index) else {
            return self.add_service(server_id, key, service_index + 1);
        };
        let attribute_key = format!("{}/{}", service.instance_key, characteristic.instance_key);
        let Some(uuid) = guid(&characteristic.uuid) else {
            let error = invalid_argument(format!("{} is not a UUID", characteristic.uuid));
            return self.fail_registration(&server_id, key, error);
        };
        let bits = properties_to_bits(&characteristic.properties);
        let local_service = server.services[service_index].provider.Service();
        let created = GattLocalCharacteristicParameters::new()
            .and_then(|parameters| {
                parameters.SetCharacteristicProperties(GattCharacteristicProperties(bits))?;
                parameters.SetReadProtectionLevel(GattProtectionLevel::Plain)?;
                parameters.SetWriteProtectionLevel(GattProtectionLevel::Plain)?;
                // No static value, so that Windows asks the backend for every read.
                local_service?.CreateCharacteristicAsync(uuid, &parameters)
            })
            .map_err(|error| winrt_error(&error, "creating a characteristic"));
        match created {
            Ok(operation) => self.spawn(operation.into_future(), move |engine, result| {
                engine.characteristic_created(
                    server_id,
                    key,
                    service_index,
                    index,
                    &attribute_key,
                    result,
                );
            }),
            Err(error) => self.fail_registration(&server_id, key, error),
        }
    }

    fn characteristic_created(
        &mut self,
        server_id: ServerId,
        key: u64,
        service_index: usize,
        index: usize,
        attribute_key: &str,
        result: windows::core::Result<GattLocalCharacteristicResult>,
    ) {
        let action = "creating a characteristic";
        let characteristic = result
            .map_err(|error| winrt_error(&error, action))
            .and_then(|result| {
                let error = result
                    .Error()
                    .map_err(|error| winrt_error(&error, action))?;
                if error != BluetoothError::Success {
                    return Err(bluetooth_error(error.0, action));
                }
                result
                    .Characteristic()
                    .map_err(|error| winrt_error(&error, action))
            });
        if self.registering(&server_id).is_none() {
            return;
        }
        let characteristic = match characteristic {
            Ok(characteristic) => characteristic,
            Err(error) => return self.fail_registration(&server_id, key, error),
        };
        let tokens = self.serve(&server_id, attribute_key, &characteristic);
        if let Some(attribute) = self
            .registering(&server_id)
            .and_then(|server| server.characteristics.get_mut(attribute_key))
        {
            attribute.characteristic = Some(characteristic);
            attribute.tokens = tokens;
        }
        self.add_characteristic(server_id, key, service_index, index + 1);
    }

    /// Registers the request handlers of a local characteristic. Windows
    /// answers a request itself, with an error, when no handler is registered.
    fn serve(
        &self,
        server_id: &ServerId,
        attribute_key: &str,
        characteristic: &GattLocalCharacteristic,
    ) -> [Option<i64>; 3] {
        let (poster, id, key) = (
            self.poster.clone(),
            server_id.clone(),
            attribute_key.to_owned(),
        );
        let read = characteristic
            .ReadRequested(&handler::<
                GattLocalCharacteristic,
                GattReadRequestedEventArgs,
            >(move |args| {
                let Some(args) = args else {
                    return;
                };
                // The deferral is taken here, before the handler returns.
                let (Ok(deferral), Ok(request)) = (args.GetDeferral(), args.GetRequestAsync())
                else {
                    return;
                };
                let central = session_device(args.Session());
                let (id, key) = (id.clone(), key.clone());
                poster.post(move |engine| {
                    engine.read_requested(id, key, central.as_deref(), deferral, request);
                });
            }))
            .ok();
        let (poster, id, key) = (
            self.poster.clone(),
            server_id.clone(),
            attribute_key.to_owned(),
        );
        let sequence = self.write_sequence.clone();
        let write = characteristic
            .WriteRequested(&handler::<
                GattLocalCharacteristic,
                GattWriteRequestedEventArgs,
            >(move |args| {
                // Numbered on arrival, so that writes answer in the order they came.
                let number = sequence.fetch_add(1, Ordering::Relaxed);
                let arrival = args.and_then(|args| {
                    Some(WriteArrival {
                        deferral: args.GetDeferral().ok()?,
                        request: args.GetRequestAsync().ok()?,
                        central: session_device(args.Session()),
                    })
                });
                let (id, key) = (id.clone(), key.clone());
                poster.post(move |engine| engine.write_arrived(number, id, key, arrival));
            }))
            .ok();
        let (poster, id, key) = (
            self.poster.clone(),
            server_id.clone(),
            attribute_key.to_owned(),
        );
        let subscribers = characteristic
            .SubscribedClientsChanged(&handler::<GattLocalCharacteristic, IInspectable>(
                move |_| {
                    let (id, key) = (id.clone(), key.clone());
                    poster.post(move |engine| engine.subscribers_changed(&id, &key));
                },
            ))
            .ok();
        [read, write, subscribers]
    }

    fn fail_registration(&mut self, server_id: &ServerId, key: u64, error: BleError) {
        self.discard_server(server_id, &cancelled());
        self.reject(key, error);
    }

    /// A deadline, `cancel` or `closeOwner` ended the `createServer`.
    pub(super) fn registration_aborted(&mut self, server_id: &ServerId) {
        if self.registering(server_id).is_some() {
            self.discard_server(server_id, &cancelled());
        }
    }

    /// Releases a server without events: its advertisement, its providers,
    /// its subscribers and its pending notifications.
    fn discard_server(&mut self, server_id: &ServerId, error: &BleError) {
        let Some(mut server) = self.servers.remove(server_id) else {
            return;
        };
        if self
            .advertisement
            .as_ref()
            .is_some_and(|advertisement| &advertisement.server_id == server_id)
        {
            if let Some(key) = self
                .advertisement
                .take()
                .and_then(|advertisement| advertisement.starting)
            {
                self.reject(key, error.clone());
            }
        }
        release(&mut server);
        for notification in server.notifications.drain() {
            if let Some(watchdog) = notification.watchdog {
                watchdog.abort();
            }
            self.reject(notification.key, error.clone());
        }
    }

    /// Closes the servers of `owner` without events.
    pub(super) fn release_servers(&mut self, owner: &OwnerId) {
        let targets: Vec<ServerId> = self
            .servers
            .iter()
            .filter(|(_, server)| &server.owner == owner)
            .map(|(server_id, _)| server_id.clone())
            .collect();
        for server_id in targets {
            self.discard_server(&server_id, &cancelled());
        }
    }

    /// The radio stopped: Windows forgets the services. A registering server
    /// fails. A ready server reports each subscriber as gone, then
    /// `criticalStateLoss`, and stays lost.
    pub(super) fn servers_lost(&mut self, error: &BleError) {
        if let Some(advertisement) = self.advertisement.take() {
            if let Some(key) = advertisement.starting {
                self.reject(key, error.clone());
            }
        }
        let reason = serde_json::to_value(error.code)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_else(|| "bluetoothOff".to_owned());
        let ids: Vec<ServerId> = self.servers.keys().cloned().collect();
        for server_id in ids {
            let Some(server) = self.servers.get_mut(&server_id) else {
                continue;
            };
            match server.state {
                ServerState::Registering => {
                    let key = server.create_key.take();
                    self.discard_server(&server_id, &cancelled());
                    if let Some(key) = key {
                        self.reject(key, error.clone());
                    }
                }
                ServerState::Ready => {
                    server.state = ServerState::Lost;
                    let owner = server.owner.clone();
                    let mut events = Vec::new();
                    for (characteristic_key, attribute) in &server.characteristics {
                        for subscriber in attribute.subscribers.values() {
                            events.push(Event::SubscriptionChanged {
                                server_id: server_id.clone(),
                                peer_id: subscriber.central.clone(),
                                characteristic_key: characteristic_key.clone(),
                                subscribed: false,
                                max_value_length: None,
                            });
                        }
                    }
                    release(server);
                    let notifications = server.notifications.drain();
                    for notification in notifications {
                        if let Some(watchdog) = notification.watchdog {
                            watchdog.abort();
                        }
                        self.reject(notification.key, disconnected("Bluetooth turned off"));
                    }
                    events.push(Event::CriticalStateLoss {
                        resource_id: server_id.to_string(),
                        reason: reason.clone(),
                    });
                    for event in events {
                        self.emit(&owner, event);
                    }
                }
                ServerState::Lost => {}
            }
        }
    }
}

// Advertising

impl Engine {
    fn advertise(
        &mut self,
        key: u64,
        server_id: &ServerId,
        index: usize,
        cut: LocalNameCut,
        name_optional: bool,
    ) {
        let checked = self
            .owned_server(key, server_id, false)
            .and_then(|()| self.check_advertiser(server_id))
            .and_then(|()| self.peripheral_error().map_or(Ok(()), Err));
        if let Err(error) = checked {
            return self.reject(key, error);
        }
        let generation = self.token();
        self.advertisement = Some(AdvertisementRecord {
            server_id: server_id.clone(),
            service_index: index,
            generation,
            starting: Some(key),
            cut,
            name_optional,
            service_data_set: false,
        });
        self.set_cleanup(key, Cleanup::Advertise(server_id.clone()));
        // Windows adds a service to its database only while its provider
        // publishes, so every other service of the server stays discoverable.
        let count = self
            .servers
            .get(server_id)
            .map_or(0, |server| server.services.len());
        for other in (0..count).filter(|other| *other != index) {
            self.publish(server_id, other, Some(Publication::Discoverable));
        }
        self.publish(server_id, index, Some(Publication::Advertised(generation)));
    }

    /// Ends the advertisement: its provider stays discoverable, so that
    /// connected centrals keep the service.
    fn stop_advertisement(&mut self, error: &BleError) {
        let Some(advertisement) = self.advertisement.take() else {
            return;
        };
        if let Some(key) = advertisement.starting {
            self.reject(key, error.clone());
        }
        self.publish(
            &advertisement.server_id,
            advertisement.service_index,
            Some(Publication::Discoverable),
        );
    }

    /// A deadline, `cancel` or `closeOwner` ended the `startAdvertising`.
    pub(super) fn advertising_aborted(&mut self, server_id: &ServerId) {
        let starting = self.advertisement.as_ref().is_some_and(|advertisement| {
            &advertisement.server_id == server_id && advertisement.starting.is_some()
        });
        if starting {
            if let Some(advertisement) = self.advertisement.as_mut() {
                advertisement.starting = None;
            }
            self.stop_advertisement(&cancelled());
        }
    }

    fn publish(&mut self, server_id: &ServerId, index: usize, desired: Option<Publication>) {
        let Some(service) = self
            .servers
            .get_mut(server_id)
            .filter(|server| server.state == ServerState::Ready)
            .and_then(|server| server.services.get_mut(index))
        else {
            return;
        };
        let step = service.publisher.want(desired);
        self.apply_step(server_id, index, step);
    }

    fn apply_step(&mut self, server_id: &ServerId, index: usize, step: Option<PublishStep>) {
        let Some(step) = step else {
            return self.watch_publication(server_id, index);
        };
        let parameters = match step {
            PublishStep::Start(publication) => Some(self.parameters(publication)),
            PublishStep::Stop => None,
        };
        let Some(service) = self
            .servers
            .get_mut(server_id)
            .and_then(|server| server.services.get_mut(index))
        else {
            return;
        };
        let called = match parameters {
            None => service.provider.StopAdvertising(),
            Some(parameters) => parameters.and_then(|parameters| {
                service.provider.StartAdvertisingWithParameters(&parameters)
            }),
        };
        match (step, called) {
            // Windows may not report the outcome, so the status is read now.
            (_, Ok(())) => self.read_provider(
                server_id,
                index,
                false,
                ProviderStatus::Other,
                BluetoothError::Success,
            ),
            (PublishStep::Stop, Err(_)) => {
                let next = service.publisher.stop_failed();
                self.apply_step(server_id, index, next);
            }
            (PublishStep::Start(publication), Err(error)) => {
                service.publisher.start_failed();
                if let Publication::Advertised(generation) = publication {
                    self.advertising_ended(generation, &winrt_error(&error, "advertising"));
                }
                self.watch_publication(server_id, index);
            }
        }
    }

    /// Reads the status of a provider again every second while a call
    /// waits for it, because Windows may never report it.
    fn watch_publication(&mut self, server_id: &ServerId, index: usize) {
        let Some(service) = self
            .servers
            .get_mut(server_id)
            .and_then(|server| server.services.get_mut(index))
        else {
            return;
        };
        if let Some(watchdog) = service.watchdog.take() {
            watchdog.abort();
        }
        if !service.publisher.is_settling() {
            return;
        }
        let id = server_id.clone();
        let watchdog = self.after(PUBLICATION_CHECK, move |engine| {
            engine.publication_due(&id, index);
        });
        if let Some(service) = self
            .servers
            .get_mut(server_id)
            .and_then(|server| server.services.get_mut(index))
        {
            service.watchdog = Some(watchdog);
        }
    }

    fn publication_due(&mut self, server_id: &ServerId, index: usize) {
        if let Some(service) = self
            .servers
            .get_mut(server_id)
            .and_then(|server| server.services.get_mut(index))
        {
            service.watchdog = None;
        }
        self.read_provider(
            server_id,
            index,
            true,
            ProviderStatus::Other,
            BluetoothError::Success,
        );
    }

    /// The parameters of a publication. The advertisement carries the local
    /// name as service data for its service UUID, as the Android backend does.
    fn parameters(
        &mut self,
        publication: Publication,
    ) -> windows::core::Result<GattServiceProviderAdvertisingParameters> {
        let parameters = GattServiceProviderAdvertisingParameters::new()?;
        parameters.SetIsDiscoverable(true)?;
        let Publication::Advertised(generation) = publication else {
            parameters.SetIsConnectable(false)?;
            return Ok(parameters);
        };
        parameters.SetIsConnectable(true)?;
        if let Some(advertisement) = self
            .advertisement
            .as_mut()
            .filter(|advertisement| advertisement.generation == generation)
        {
            advertisement.service_data_set = advertisement.cut.included()
                && buffer(&advertisement.cut.bytes)
                    .and_then(|name| parameters.SetServiceData(&name))
                    .is_ok();
        }
        Ok(parameters)
    }

    /// Reads the status of the provider of service `index` and feeds it to
    /// its publisher, `later` when the watchdog reads it. An event only
    /// prompts the read: its `reported` status counts when the read fails.
    /// Then it reports a start or an end, and takes the next step.
    fn read_provider(
        &mut self,
        server_id: &ServerId,
        index: usize,
        later: bool,
        reported: ProviderStatus,
        error: BluetoothError,
    ) {
        let Some(service) = self
            .servers
            .get_mut(server_id)
            .and_then(|server| server.services.get_mut(index))
        else {
            return;
        };
        let status = service
            .provider
            .AdvertisementStatus()
            .map_or(reported, provider_status);
        let (published, step) = service.publisher.check(status, later);
        match published {
            Some(Published::Started {
                publication: Publication::Advertised(generation),
                all_data,
            }) => self.advertising_started(generation, all_data),
            Some(Published::Ended(Publication::Advertised(generation))) => {
                self.advertising_ended(generation, &bluetooth_error(error.0, "advertising"));
            }
            _ => {}
        }
        self.apply_step(server_id, index, step);
    }

    /// Resolves the `startAdvertising`. When Windows left the name out and
    /// the name is required, the advertisement stops and the start rejects.
    fn advertising_started(&mut self, generation: u64, all_data: bool) {
        let Some(advertisement) = self
            .advertisement
            .as_mut()
            .filter(|advertisement| advertisement.generation == generation)
        else {
            return;
        };
        let Some(key) = advertisement.starting.take() else {
            return;
        };
        let report = advertising_report(
            &advertisement.cut,
            all_data && advertisement.service_data_set,
            advertisement.name_optional,
        );
        match report {
            Ok(report) => {
                self.set_cleanup(key, Cleanup::None);
                self.resolve(key, Reply::AdvertisingStarted(report));
            }
            Err(error) => {
                self.reject(key, error);
                self.stop_advertisement(&cancelled());
            }
        }
    }

    /// Windows refused or ended the advertisement.
    fn advertising_ended(&mut self, generation: u64, error: &BleError) {
        if self
            .advertisement
            .as_ref()
            .is_some_and(|advertisement| advertisement.generation == generation)
        {
            self.stop_advertisement(error);
        }
    }
}

// Notifications

impl Engine {
    /// Sends the next notification of the server. One is outstanding at a
    /// time, and a notify resolves only when Windows took the value: this is
    /// the flow control of the host.
    fn send_notifications(&mut self, server_id: &ServerId) {
        loop {
            let Some(server) = self
                .servers
                .get_mut(server_id)
                .filter(|server| server.state == ServerState::Ready)
            else {
                return;
            };
            let Some(notification) = server.notifications.start_next() else {
                return;
            };
            let (token, key, deadline_at) = (
                notification.token,
                notification.key,
                notification.deadline_at,
            );
            if !self.pending(key) {
                self.notification_done(server_id, token, Err(cancelled()), false);
                continue;
            }
            match self.send_notification(server_id, token) {
                Ok(()) => {
                    let delay =
                        deadline_at.saturating_duration_since(Instant::now()) + NOTIFICATION_GRACE;
                    let id = server_id.clone();
                    let watchdog = self.after(delay, move |engine| {
                        engine.notification_done(&id, token, Err(timeout()), true);
                    });
                    if let Some(notification) = self
                        .servers
                        .get_mut(server_id)
                        .and_then(|server| server.notifications.in_flight_mut())
                    {
                        notification.watchdog = Some(watchdog);
                    }
                    return;
                }
                Err(error) => self.notification_done(server_id, token, Err(error), false),
            }
        }
    }

    fn send_notification(&mut self, server_id: &ServerId, token: u64) -> BleResult<()> {
        let server = self
            .servers
            .get(server_id)
            .ok_or_else(|| disconnected("the server closed"))?;
        let notification = server
            .notifications
            .in_flight()
            .ok_or_else(|| disconnected("the server closed"))?;
        let attribute = server.characteristics.get(&notification.characteristic_key);
        let target = attribute.and_then(|attribute| {
            let subscriber = attribute
                .subscribers
                .values()
                .find(|subscriber| subscriber.central == notification.central)?;
            Some((attribute.characteristic.clone()?, subscriber.client.clone()))
        });
        let (characteristic, client) =
            target.ok_or_else(|| disconnected("the central unsubscribed"))?;
        let operation = buffer(&notification.value)
            .and_then(|value| characteristic.NotifyValueForSubscribedClientAsync(&value, &client))
            .map_err(|error| winrt_error(&error, "the notification"))?;
        let id = server_id.clone();
        self.spawn(
            async move {
                let failed = |error: windows::core::Error| winrt_error(&error, "the notification");
                let result = operation.await.map_err(failed)?;
                let status = result.Status().map_err(failed)?;
                if status == GattCommunicationStatus::Success {
                    Ok(())
                } else {
                    Err(communication_error(
                        status.0,
                        att_error(result.ProtocolError()),
                        "the notification",
                    ))
                }
            },
            move |engine, outcome| engine.notification_done(&id, token, outcome, true),
        );
        Ok(())
    }

    /// Ends the notification in flight when it is `token`, then sends the next one.
    fn notification_done(
        &mut self,
        server_id: &ServerId,
        token: u64,
        outcome: BleResult<()>,
        next: bool,
    ) {
        let Some(notification) = self.servers.get_mut(server_id).and_then(|server| {
            server
                .notifications
                .finish(|notification| notification.token == token)
        }) else {
            return;
        };
        if let Some(watchdog) = notification.watchdog {
            watchdog.abort();
        }
        match outcome {
            Ok(()) => self.resolve(notification.key, Reply::Empty),
            Err(error) => self.reject(notification.key, error),
        }
        if next {
            self.send_notifications(server_id);
        }
    }

    /// A deadline, `cancel` or `closeOwner` ended a `notify`. A queued one
    /// leaves the queue. One in flight keeps its place until Windows answers.
    pub(super) fn notification_aborted(&mut self, server_id: &ServerId, token: u64) {
        if let Some(server) = self.servers.get_mut(server_id) {
            server
                .notifications
                .remove_waiting(|notification| notification.token == token);
        }
    }
}

// Requests from centrals

impl Engine {
    fn live_attribute(
        &self,
        server_id: &ServerId,
        characteristic_key: &str,
    ) -> Option<&LocalAttribute> {
        self.servers
            .get(server_id)
            .filter(|server| server.state == ServerState::Ready)
            .and_then(|server| server.characteristics.get(characteristic_key))
            .filter(|attribute| attribute.characteristic.is_some())
    }

    fn central_id(&mut self, device: &str) -> PeerId {
        PeerId::new(self.centrals.id_for(&device.to_owned(), &mut self.ids))
    }

    fn read_requested(
        &mut self,
        server_id: ServerId,
        characteristic_key: String,
        central: Option<&str>,
        deferral: Deferral,
        request: IAsyncOperation<GattReadRequest>,
    ) {
        if let Some(device) = central {
            self.central_id(device);
        }
        self.spawn(request.into_future(), move |engine, request| {
            engine.answer_read(&server_id, &characteristic_key, &deferral, request.ok());
        });
    }

    /// Answers a read from the stored value, from the offset of the request.
    fn answer_read(
        &mut self,
        server_id: &ServerId,
        characteristic_key: &str,
        deferral: &Deferral,
        request: Option<GattReadRequest>,
    ) {
        if let Some(request) = request {
            let offset = request.Offset().unwrap_or_default() as usize;
            let answer = match self.live_attribute(server_id, characteristic_key) {
                Some(attribute) => read_answer(attribute.properties.read, &attribute.value, offset)
                    .map(<[u8]>::to_vec),
                None => Err(att::ATTRIBUTE_NOT_FOUND),
            };
            let _ = match answer {
                Ok(value) => buffer(&value).and_then(|value| request.RespondWithValue(&value)),
                Err(code) => request.RespondWithProtocolError(code),
            };
        }
        let _ = deferral.Complete();
    }

    fn write_arrived(
        &mut self,
        number: u64,
        server_id: ServerId,
        characteristic_key: String,
        arrival: Option<WriteArrival>,
    ) {
        match arrival {
            None => self.writes.fill(number, None),
            Some(arrival) => {
                self.writes.expect(number);
                self.spawn(arrival.request.into_future(), move |engine, request| {
                    engine.writes.fill(
                        number,
                        Some(ReadyWrite {
                            server_id,
                            characteristic_key,
                            central: arrival.central,
                            deferral: arrival.deferral,
                            request: request.ok(),
                        }),
                    );
                    engine.answer_writes();
                });
            }
        }
        self.answer_writes();
    }

    fn answer_writes(&mut self) {
        while let Some(write) = self.writes.pop() {
            self.answer_write(&write);
        }
    }

    /// Validates one write, answers it when it needs a response, then emits
    /// `serverWrite`. Windows raises one request per write and gives no
    /// prepare or execute boundary, so each request is a whole write.
    fn answer_write(&mut self, write: &ReadyWrite) {
        if let Some(request) = &write.request {
            let with_response = request.Option().ok() == Some(GattWriteOption::WriteWithResponse);
            let offset = request.Offset().unwrap_or_default() as usize;
            let value = request.Value().and_then(|value| bytes(&value));
            let peer_id = write
                .central
                .as_deref()
                .map(|device| self.central_id(device));
            let outcome = match (
                self.live_attribute(&write.server_id, &write.characteristic_key),
                value,
            ) {
                (None, _) => Err(att::ATTRIBUTE_NOT_FOUND),
                (Some(_), Err(_)) => Err(att::UNLIKELY_ERROR),
                (Some(attribute), Ok(value)) => {
                    match write_error(
                        &attribute.properties,
                        attribute.max_value_length,
                        offset,
                        value.len(),
                    ) {
                        Some(code) => Err(code),
                        None => Ok(value),
                    }
                }
            };
            match outcome {
                Ok(value) => {
                    if with_response {
                        let _ = request.Respond();
                    }
                    if let Some(owner) = self
                        .servers
                        .get(&write.server_id)
                        .map(|server| server.owner.clone())
                    {
                        self.emit(
                            &owner,
                            Event::ServerWrite {
                                server_id: write.server_id.clone(),
                                peer_id,
                                characteristic_key: write.characteristic_key.clone(),
                                value_base64: BASE64.encode(value),
                            },
                        );
                    }
                }
                Err(code) => {
                    if with_response {
                        let _ = request.RespondWithProtocolError(code);
                    }
                }
            }
        }
        let _ = write.deferral.Complete();
    }

    /// Diffs the subscribed clients of a characteristic with the known ones,
    /// and reports each change with the notification size of its central.
    fn subscribers_changed(&mut self, server_id: &ServerId, characteristic_key: &str) {
        let Some(attribute) = self.live_attribute(server_id, characteristic_key) else {
            return;
        };
        let clients = attribute
            .characteristic
            .as_ref()
            .and_then(|characteristic| characteristic.SubscribedClients().ok())
            .and_then(|clients| items(&clients).ok())
            .unwrap_or_default();
        let mut after = BTreeMap::new();
        let mut objects = HashMap::new();
        for client in clients {
            let Some(device) = session_device(client.Session()) else {
                continue;
            };
            let length = notification_length(client.MaxNotificationSize().unwrap_or(20));
            after.insert(device.clone(), length);
            objects.insert(device, client);
        }
        let before: BTreeMap<String, u32> = attribute
            .subscribers
            .iter()
            .map(|(device, subscriber)| (device.clone(), subscriber.length))
            .collect();
        for change in subscriber_changes(&before, &after) {
            match change {
                SubscriberChange::Unsubscribed(device) => {
                    self.unsubscribed(server_id, characteristic_key, &device);
                }
                SubscriberChange::Subscribed(device, length) => {
                    if let Some(client) = objects.remove(&device) {
                        self.subscribed(server_id, characteristic_key, device, client, length);
                    }
                }
                SubscriberChange::Resized(device, length) => {
                    if let Some(subscriber) = self
                        .servers
                        .get_mut(server_id)
                        .and_then(|server| server.characteristics.get_mut(characteristic_key))
                        .and_then(|attribute| attribute.subscribers.get_mut(&device))
                    {
                        subscriber.length = length;
                        let peer_id = subscriber.central.clone();
                        self.subscription_changed(
                            server_id,
                            characteristic_key,
                            peer_id,
                            Some(length),
                        );
                    }
                }
            }
        }
    }

    fn subscribed(
        &mut self,
        server_id: &ServerId,
        characteristic_key: &str,
        device: String,
        client: GattSubscribedClient,
        length: u32,
    ) {
        let central = self.central_id(&device);
        let (poster, id, key) = (
            self.poster.clone(),
            server_id.clone(),
            characteristic_key.to_owned(),
        );
        let size_token = client
            .MaxNotificationSizeChanged(&handler::<GattSubscribedClient, IInspectable>(move |_| {
                let (id, key) = (id.clone(), key.clone());
                poster.post(move |engine| engine.subscribers_changed(&id, &key));
            }))
            .ok();
        if let Some(attribute) = self
            .servers
            .get_mut(server_id)
            .and_then(|server| server.characteristics.get_mut(characteristic_key))
        {
            attribute.subscribers.insert(
                device,
                Subscriber {
                    central: central.clone(),
                    client,
                    length,
                    size_token,
                },
            );
        }
        self.subscription_changed(server_id, characteristic_key, central, Some(length));
    }

    /// A notification still queued for a central that unsubscribed rejects with `disconnected`.
    fn unsubscribed(&mut self, server_id: &ServerId, characteristic_key: &str, device: &str) {
        let Some(server) = self.servers.get_mut(server_id) else {
            return;
        };
        let Some(subscriber) = server
            .characteristics
            .get_mut(characteristic_key)
            .and_then(|attribute| attribute.subscribers.remove(device))
        else {
            return;
        };
        if let Some(token) = subscriber.size_token {
            let _ = subscriber.client.RemoveMaxNotificationSizeChanged(token);
        }
        let dropped = server.notifications.remove_waiting(|notification| {
            notification.central == subscriber.central
                && notification.characteristic_key == characteristic_key
        });
        for notification in dropped {
            self.reject(notification.key, disconnected("the central unsubscribed"));
        }
        self.subscription_changed(server_id, characteristic_key, subscriber.central, None);
    }

    fn subscription_changed(
        &self,
        server_id: &ServerId,
        characteristic_key: &str,
        peer_id: PeerId,
        max_value_length: Option<u32>,
    ) {
        let Some(server) = self.servers.get(server_id) else {
            return;
        };
        self.emit(
            &server.owner,
            Event::SubscriptionChanged {
                server_id: server_id.clone(),
                peer_id,
                characteristic_key: characteristic_key.to_owned(),
                subscribed: max_value_length.is_some(),
                max_value_length,
            },
        );
    }
}

/// Releases the `WinRT` objects of a server: its providers stop publishing,
/// and every handler is removed.
fn release(server: &mut Server) {
    for service in server.services.drain(..) {
        if let Some(watchdog) = service.watchdog {
            watchdog.abort();
        }
        let _ = service.provider.StopAdvertising();
        if let Some(token) = service.status_token {
            let _ = service.provider.RemoveAdvertisementStatusChanged(token);
        }
    }
    for attribute in server.characteristics.values_mut() {
        if let Some(characteristic) = attribute.characteristic.take() {
            let [read, write, subscribers] = attribute.tokens;
            if let Some(token) = read {
                let _ = characteristic.RemoveReadRequested(token);
            }
            if let Some(token) = write {
                let _ = characteristic.RemoveWriteRequested(token);
            }
            if let Some(token) = subscribers {
                let _ = characteristic.RemoveSubscribedClientsChanged(token);
            }
        }
        for subscriber in std::mem::take(&mut attribute.subscribers).into_values() {
            if let Some(token) = subscriber.size_token {
                let _ = subscriber.client.RemoveMaxNotificationSizeChanged(token);
            }
        }
    }
}

fn provider_status(status: GattServiceProviderAdvertisementStatus) -> ProviderStatus {
    match status {
        GattServiceProviderAdvertisementStatus::Created => ProviderStatus::Created,
        GattServiceProviderAdvertisementStatus::Started => {
            ProviderStatus::Started { all_data: true }
        }
        GattServiceProviderAdvertisementStatus::StartedWithoutAllAdvertisementData => {
            ProviderStatus::Started { all_data: false }
        }
        GattServiceProviderAdvertisementStatus::Stopped => ProviderStatus::Stopped,
        GattServiceProviderAdvertisementStatus::Aborted => ProviderStatus::Aborted,
        _ => ProviderStatus::Other,
    }
}
