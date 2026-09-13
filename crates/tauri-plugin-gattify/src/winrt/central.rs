use std::{
    collections::HashMap,
    future::IntoFuture,
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use tokio::task::AbortHandle;
use windows::{
    core::IInspectable,
    Devices::Bluetooth::{
        BluetoothAddressType, BluetoothCacheMode, BluetoothConnectionStatus, BluetoothLEDevice,
        GenericAttributeProfile::{
            GattCharacteristic, GattClientCharacteristicConfigurationDescriptorValue,
            GattCommunicationStatus, GattDeviceService, GattDeviceServicesResult, GattReadResult,
            GattSession, GattSessionStatus, GattSessionStatusChangedEventArgs,
            GattValueChangedEventArgs, GattWriteOption, GattWriteResult,
        },
    },
};
use windows_future::IAsyncOperation;

use super::{
    convert::{att_error, buffer, bytes, handler, items, uuid, winrt_error, winrt_or_missing},
    engine::{busy, cancelled, decode, disconnected, Cleanup, Engine},
    gatt::{
        check_access, check_write_length, link_limits, properties_from_bits, Access, Mode,
        DEFAULT_ATT_MTU,
    },
    ids::HandleTable,
    queue::ProcedureQueue,
    status::communication_error,
};
use crate::{
    BleError, BleResult, CharacteristicHandle, CharacteristicInstance, ConnectionId, ErrorCode,
    Event, OwnerId, Reply, ServiceHandle, ServiceInstance, SubscriptionId, WriteType,
};

/// How long a connect waits for the MTU exchange once the link is up.
const LINK_LIMIT_WAIT: Duration = Duration::from_secs(1);
/// How long a disconnect waits for the link to close.
const DISCONNECT_WAIT: Duration = Duration::from_secs(2);
/// The deadline of a GATT procedure that no operation waits for.
const PROCEDURE_DEADLINE: Duration = Duration::from_secs(5);

/// The connection bookkeeping of one remote device.
#[derive(Default)]
pub(super) struct DeviceLink {
    /// From the last scan result, so that a random address connects too.
    pub(super) address_type: Option<BluetoothAddressType>,
    connection: Option<ConnectionId>,
    /// Connects that wait for a closing connection to this device.
    waiting: Vec<u64>,
}

/// An active subscription, as Rust knows it.
pub(super) struct SubscriptionRef {
    pub(super) owner: OwnerId,
    connection_id: ConnectionId,
    handle: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    Connecting,
    Connected,
    Closing,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SubscriptionPhase {
    /// The descriptor write runs. The characteristic is reserved.
    Enabling,
    Active,
    /// An unsubscribe runs. No value is emitted meanwhile.
    Disabling,
}

struct Subscription {
    id: SubscriptionId,
    characteristic: GattCharacteristic,
    value_token: Option<i64>,
    phase: SubscriptionPhase,
}

#[derive(Clone)]
enum Call {
    Discover,
    Read(GattCharacteristic),
    Write(GattCharacteristic, Vec<u8>, WriteType),
    Subscribe(String, Mode),
    Unsubscribe(String),
    /// Undoes the descriptor write of a cancelled subscribe.
    Rollback(String),
}

/// One GATT procedure. `key` is the operation that waits for it, if any.
struct Procedure {
    token: u64,
    key: Option<u64>,
    call: Call,
    deadline_at: Instant,
    watchdog: Option<AbortHandle>,
}

enum Done {
    Services(Vec<(GattDeviceService, Vec<GattCharacteristic>)>),
    Bytes(Vec<u8>),
    Empty,
}

/// How a link ends. `wait` holds the reply of a disconnect until the link
/// closes, at most that long.
struct Closing {
    emit: bool,
    error: BleError,
    wait: Option<Duration>,
}

impl Closing {
    fn silent(error: BleError) -> Self {
        Self {
            emit: false,
            error,
            wait: None,
        }
    }

    fn lost(message: &str) -> Self {
        Self {
            emit: true,
            error: disconnected(message),
            wait: None,
        }
    }
}

pub(super) struct Connection {
    pub(super) owner: OwnerId,
    device_id: String,
    phase: Phase,
    connect_key: Option<u64>,
    device: Option<BluetoothLEDevice>,
    status_token: Option<i64>,
    session: Option<GattSession>,
    session_tokens: [Option<i64>; 2],
    link_up: bool,
    limit_timer: Option<AbortHandle>,
    close_timer: Option<AbortHandle>,
    disconnects: Vec<u64>,
    handles: HandleTable<GattCharacteristic>,
    /// Every service object a discovery returned. They close with the link.
    services: Vec<GattDeviceService>,
    subscriptions: HashMap<String, Subscription>,
    queue: ProcedureQueue<Procedure>,
}

impl Connection {
    pub(super) fn is_connected(&self) -> bool {
        self.phase == Phase::Connected
    }

    pub(super) fn is_closing(&self) -> bool {
        self.phase == Phase::Closing
    }

    /// The ATT MTU of the link: `GattSession.MaxPduSize`.
    fn mtu(&self) -> u16 {
        self.session
            .as_ref()
            .and_then(|session| session.MaxPduSize().ok())
            .unwrap_or(DEFAULT_ATT_MTU)
    }
}

// Commands

impl Engine {
    pub(super) fn connect(&mut self, key: u64, device_id: String) {
        let Some(owner) = self.owner(key) else {
            return;
        };
        if self.devices.key_of(&device_id).is_none()
            || !self.devices.seen_by_family_of(&device_id, owner.as_str())
        {
            return self.reject(key, BleError::invalid_handle(device_id.as_str()));
        }
        let existing = self
            .device_links
            .get(&device_id)
            .and_then(|link| link.connection.clone());
        if let Some(existing) = existing {
            if self
                .connections
                .get(&existing)
                .is_some_and(Connection::is_closing)
            {
                // The attempt runs when the closing link has let the device go.
                if let Some(link) = self.device_links.get_mut(&device_id) {
                    link.waiting.push(key);
                }
                return;
            }
            return self.reject(key, busy(format!("{device_id} already has a connection")));
        }
        self.when_adapter(key, true, move |engine, key| {
            engine.open_link(key, device_id);
        });
    }

    pub(super) fn disconnect(&mut self, key: u64, connection_id: &ConnectionId) {
        let owner = self.owner(key);
        let Some(link) = self
            .connections
            .get_mut(connection_id)
            .filter(|link| Some(&link.owner) == owner.as_ref() && link.phase != Phase::Connecting)
        else {
            return self.reject(key, BleError::invalid_handle(connection_id.clone()));
        };
        link.disconnects.push(key);
        let wait = self.deadline_at(key).map_or(DISCONNECT_WAIT, |deadline| {
            deadline
                .saturating_duration_since(Instant::now())
                .min(DISCONNECT_WAIT)
        });
        self.close_link(
            connection_id,
            &Closing {
                emit: false,
                error: disconnected("disconnect ended the procedure"),
                wait: Some(wait),
            },
        );
    }

    pub(super) fn discover_services(&mut self, key: u64, connection_id: &ConnectionId) {
        if let Err(error) = self.connected_link(key, connection_id) {
            return self.reject(key, error);
        }
        self.enqueue(connection_id, key, Call::Discover);
    }

    pub(super) fn read(&mut self, key: u64, connection_id: &ConnectionId, handle: &str) {
        let checked = self
            .link_characteristic(key, connection_id, handle)
            .and_then(|characteristic| {
                check_access(&properties(&characteristic), Access::Read)?;
                Ok(characteristic)
            });
        match checked {
            Ok(characteristic) => self.enqueue(connection_id, key, Call::Read(characteristic)),
            Err(error) => self.reject(key, error),
        }
    }

    pub(super) fn write(
        &mut self,
        key: u64,
        connection_id: &ConnectionId,
        handle: &str,
        value_base64: &str,
        write_type: WriteType,
    ) {
        let checked = self
            .link_characteristic(key, connection_id, handle)
            .and_then(|characteristic| {
                let value = decode(value_base64)?;
                let mtu = self
                    .connections
                    .get(connection_id)
                    .map_or(DEFAULT_ATT_MTU, Connection::mtu);
                check_write_length(value.len(), write_type, mtu)?;
                check_access(&properties(&characteristic), Access::Write(write_type))?;
                Ok(Call::Write(characteristic, value, write_type))
            });
        match checked {
            Ok(call) => self.enqueue(connection_id, key, call),
            Err(error) => self.reject(key, error),
        }
    }

    pub(super) fn subscribe(&mut self, key: u64, connection_id: &ConnectionId, handle: &str) {
        let checked = self
            .link_characteristic(key, connection_id, handle)
            .and_then(|characteristic| {
                let mode = check_access(&properties(&characteristic), Access::Subscribe)?;
                Ok((characteristic, mode.unwrap_or(Mode::Notify)))
            });
        let (characteristic, mode) = match checked {
            Ok(checked) => checked,
            Err(error) => return self.reject(key, error),
        };
        if self
            .connections
            .get(connection_id)
            .is_some_and(|link| link.subscriptions.contains_key(handle))
        {
            return self.reject(
                key,
                busy("the characteristic already has a subscription on this connection"),
            );
        }
        let id = SubscriptionId::new(self.ids.next("subscription"));
        let Some(link) = self.connections.get_mut(connection_id) else {
            return;
        };
        // The reservation makes another subscribe busy until this one ends,
        // including the rollback of a cancelled one.
        link.subscriptions.insert(
            handle.to_owned(),
            Subscription {
                id,
                characteristic,
                value_token: None,
                phase: SubscriptionPhase::Enabling,
            },
        );
        self.enqueue(connection_id, key, Call::Subscribe(handle.to_owned(), mode));
    }

    pub(super) fn unsubscribe(&mut self, key: u64, subscription_id: &SubscriptionId) {
        let owner = self.owner(key);
        let Some((connection_id, handle)) = self
            .subscriptions
            .get(subscription_id)
            .filter(|subscription| Some(&subscription.owner) == owner.as_ref())
            .map(|subscription| {
                (
                    subscription.connection_id.clone(),
                    subscription.handle.clone(),
                )
            })
        else {
            return self.reject(key, BleError::invalid_handle(subscription_id.clone()));
        };
        let Some(subscription) = self
            .connections
            .get_mut(&connection_id)
            .and_then(|link| link.subscriptions.get_mut(&handle))
        else {
            return self.reject(key, BleError::invalid_handle(subscription_id.clone()));
        };
        if subscription.phase == SubscriptionPhase::Disabling {
            return self.reject(key, busy(format!("{subscription_id} is already ending")));
        }
        subscription.phase = SubscriptionPhase::Disabling;
        self.enqueue(&connection_id, key, Call::Unsubscribe(handle));
    }

    fn connected_link(&self, key: u64, connection_id: &ConnectionId) -> BleResult<&Connection> {
        let owner = self.owner(key);
        let link = self
            .connections
            .get(connection_id)
            .filter(|link| Some(&link.owner) == owner.as_ref() && link.phase != Phase::Connecting)
            .ok_or_else(|| BleError::invalid_handle(connection_id.clone()))?;
        if link.phase == Phase::Closing {
            return Err(disconnected("the connection is closing"));
        }
        if let Some(error) = self.radio_error() {
            return Err(error);
        }
        Ok(link)
    }

    fn link_characteristic(
        &self,
        key: u64,
        connection_id: &ConnectionId,
        handle: &str,
    ) -> BleResult<GattCharacteristic> {
        self.connected_link(key, connection_id)?
            .handles
            .get(handle)
            .cloned()
            .ok_or_else(|| BleError::invalid_handle(handle))
    }
}

// Links

impl Engine {
    fn open_link(&mut self, key: u64, device_id: String) {
        let Some(owner) = self.owner(key) else {
            return;
        };
        let Some(&address) = self.devices.key_of(&device_id) else {
            return self.reject(key, BleError::invalid_handle(device_id.as_str()));
        };
        if self
            .device_links
            .get(&device_id)
            .is_some_and(|link| link.connection.is_some())
        {
            return self.reject(key, busy(format!("{device_id} already has a connection")));
        }
        let connection_id = ConnectionId::new(self.ids.next("connection"));
        let link = self.device_links.entry(device_id.clone()).or_default();
        link.connection = Some(connection_id.clone());
        let address_type = link.address_type;
        self.connections.insert(
            connection_id.clone(),
            Connection {
                owner,
                device_id,
                phase: Phase::Connecting,
                connect_key: Some(key),
                device: None,
                status_token: None,
                session: None,
                session_tokens: [None, None],
                link_up: false,
                limit_timer: None,
                close_timer: None,
                disconnects: Vec::new(),
                handles: HandleTable::new(connection_id.as_str()),
                services: Vec::new(),
                subscriptions: HashMap::new(),
                queue: ProcedureQueue::default(),
            },
        );
        self.set_cleanup(key, Cleanup::Connect(connection_id.clone()));
        let opening = match address_type {
            Some(address_type) => {
                BluetoothLEDevice::FromBluetoothAddressWithBluetoothAddressTypeAsync(
                    address,
                    address_type,
                )
            }
            None => BluetoothLEDevice::FromBluetoothAddressAsync(address),
        };
        match opening {
            Ok(operation) => self.spawn(operation.into_future(), move |engine, device| {
                engine.device_opened(&connection_id, device);
            }),
            Err(error) => {
                self.fail_connect(&connection_id, winrt_error(&error, "opening the device"));
            }
        }
    }

    fn connecting(&mut self, connection_id: &ConnectionId) -> Option<&mut Connection> {
        self.connections
            .get_mut(connection_id)
            .filter(|link| link.phase == Phase::Connecting)
    }

    fn device_opened(
        &mut self,
        connection_id: &ConnectionId,
        device: windows::core::Result<BluetoothLEDevice>,
    ) {
        let device = match device {
            Ok(device) => device,
            Err(error) => {
                let error = winrt_or_missing(
                    &error,
                    "opening the device",
                    ErrorCode::Disconnected,
                    "Windows could not find the device",
                );
                return self.fail_connect(connection_id, error);
            }
        };
        if self.connecting(connection_id).is_none() {
            let _ = device.Close();
            return;
        }
        let poster = self.poster.clone();
        let id = connection_id.clone();
        let status_token = device
            .ConnectionStatusChanged(&handler::<BluetoothLEDevice, IInspectable>(move |_| {
                let id = id.clone();
                poster.post(move |engine| engine.link_status_changed(&id));
            }))
            .ok();
        let session = device
            .BluetoothDeviceId()
            .and_then(|device_id| GattSession::FromDeviceIdAsync(&device_id));
        if let Some(link) = self.connecting(connection_id) {
            link.device = Some(device);
            link.status_token = status_token;
        }
        match session {
            Ok(operation) => {
                let id = connection_id.clone();
                self.spawn(operation.into_future(), move |engine, session| {
                    engine.session_opened(&id, session);
                });
            }
            Err(error) => {
                self.fail_connect(
                    connection_id,
                    winrt_error(&error, "opening the GATT session"),
                );
            }
        }
    }

    /// Windows has no call that connects. The GATT session connects and keeps
    /// the link while `MaintainConnection` is true.
    fn session_opened(
        &mut self,
        connection_id: &ConnectionId,
        session: windows::core::Result<GattSession>,
    ) {
        let session = match session {
            Ok(session) => session,
            Err(error) => {
                let error = winrt_or_missing(
                    &error,
                    "opening the GATT session",
                    ErrorCode::Disconnected,
                    "Windows could not open a GATT session",
                );
                return self.fail_connect(connection_id, error);
            }
        };
        if self.connecting(connection_id).is_none() {
            let _ = session.Close();
            return;
        }
        if !session.CanMaintainConnection().unwrap_or(false) {
            let _ = session.Close();
            return self.fail_connect(
                connection_id,
                BleError::unsupported("Windows cannot keep a connection to this device"),
            );
        }
        let poster = self.poster.clone();
        let id = connection_id.clone();
        let status_token = session
            .SessionStatusChanged(&handler::<GattSession, GattSessionStatusChangedEventArgs>(
                move |args| {
                    if let Some(status) = args.and_then(|args| args.Status().ok()) {
                        let id = id.clone();
                        poster.post(move |engine| engine.session_status_changed(&id, status));
                    }
                },
            ))
            .ok();
        let poster = self.poster.clone();
        let id = connection_id.clone();
        let mtu_token = session
            .MaxPduSizeChanged(&handler::<GattSession, IInspectable>(move |_| {
                let id = id.clone();
                poster.post(move |engine| engine.mtu_changed(&id));
            }))
            .ok();
        let maintained = session.SetMaintainConnection(true);
        let active = session.SessionStatus().ok() == Some(GattSessionStatus::Active);
        if let Some(link) = self.connecting(connection_id) {
            link.session = Some(session);
            link.session_tokens = [status_token, mtu_token];
        }
        if let Err(error) = maintained {
            return self.fail_connect(connection_id, winrt_error(&error, "connecting"));
        }
        if active {
            self.link_up(connection_id);
        }
    }

    fn session_status_changed(&mut self, connection_id: &ConnectionId, status: GattSessionStatus) {
        if status == GattSessionStatus::Active {
            if self.connecting(connection_id).is_some() {
                self.link_up(connection_id);
            }
        } else {
            self.link_down(connection_id);
        }
    }

    fn link_status_changed(&mut self, connection_id: &ConnectionId) {
        let down = self
            .connections
            .get(connection_id)
            .and_then(|link| link.device.as_ref())
            .and_then(|device| device.ConnectionStatus().ok())
            == Some(BluetoothConnectionStatus::Disconnected);
        if down {
            self.link_down(connection_id);
        }
    }

    /// The session closed or the device disconnected.
    fn link_down(&mut self, connection_id: &ConnectionId) {
        let Some(link) = self.connections.get(connection_id) else {
            return;
        };
        match link.phase {
            // Before the link is up, Windows still tries to connect.
            Phase::Connecting if link.link_up => self.fail_connect(
                connection_id,
                disconnected("the link closed before the connection was ready"),
            ),
            Phase::Connecting => {}
            Phase::Connected => self.close_link(connection_id, &Closing::lost("the link closed")),
            Phase::Closing => self.finish_close(connection_id),
        }
    }

    /// Windows exchanges the MTU after the link is up. The connect waits up
    /// to 1 s for it, as the iOS connect does.
    fn link_up(&mut self, connection_id: &ConnectionId) {
        let Some(link) = self.connecting(connection_id) else {
            return;
        };
        if link.link_up {
            return;
        }
        link.link_up = true;
        if link.mtu() > DEFAULT_ATT_MTU {
            return self.finish_connect(connection_id);
        }
        let id = connection_id.clone();
        let timer = self.after(LINK_LIMIT_WAIT, move |engine| engine.finish_connect(&id));
        if let Some(link) = self.connecting(connection_id) {
            link.limit_timer = Some(timer);
        }
    }

    fn mtu_changed(&mut self, connection_id: &ConnectionId) {
        if self
            .connecting(connection_id)
            .is_some_and(|link| link.link_up && link.mtu() > DEFAULT_ATT_MTU)
        {
            self.finish_connect(connection_id);
        }
    }

    fn finish_connect(&mut self, connection_id: &ConnectionId) {
        let Some(link) = self.connecting(connection_id).filter(|link| link.link_up) else {
            return;
        };
        link.phase = Phase::Connected;
        if let Some(timer) = link.limit_timer.take() {
            timer.abort();
        }
        let limits = link_limits(link.mtu());
        if let Some(key) = link.connect_key.take() {
            self.resolve(
                key,
                Reply::Connected {
                    connection_id: connection_id.clone(),
                    limits,
                },
            );
        }
    }

    fn fail_connect(&mut self, connection_id: &ConnectionId, error: BleError) {
        let Some(link) = self.connecting(connection_id) else {
            return;
        };
        if let Some(key) = link.connect_key.take() {
            self.reject(key, error.clone());
        }
        self.close_link(connection_id, &Closing::silent(error));
    }

    /// A deadline, `cancel` or `closeOwner` ended the connect: the attempt stops.
    pub(super) fn connect_aborted(&mut self, connection_id: &ConnectionId) {
        if let Some(link) = self.connecting(connection_id) {
            link.connect_key = None;
            self.close_link(connection_id, &Closing::silent(cancelled()));
        }
    }

    /// Ends a link: rejects its procedures, ends its subscriptions without
    /// events, and releases every `WinRT` object of it so that Windows can
    /// drop the link. The session stays until it reports the close, or
    /// until the wait of a disconnect ends.
    fn close_link(&mut self, connection_id: &ConnectionId, closing: &Closing) {
        let Some(link) = self.connections.get_mut(connection_id) else {
            return;
        };
        if link.phase == Phase::Closing {
            if closing.wait.is_none() {
                self.finish_close(connection_id);
            }
            return;
        }
        let was_connected = link.phase == Phase::Connected;
        link.phase = Phase::Closing;
        if let Some(timer) = link.limit_timer.take() {
            timer.abort();
        }
        let owner = link.owner.clone();
        let connect_key = link.connect_key.take();
        let procedures = link.queue.drain();
        let subscriptions: Vec<Subscription> = link
            .subscriptions
            .drain()
            .map(|(_, subscription)| subscription)
            .collect();
        link.handles.forget_attributes();
        for service in link.services.drain(..) {
            let _ = service.Close();
        }
        if let Some(device) = link.device.take() {
            if let Some(token) = link.status_token.take() {
                let _ = device.RemoveConnectionStatusChanged(token);
            }
            let _ = device.Close();
        }
        let active = link.session.as_ref().is_some_and(|session| {
            let _ = session.SetMaintainConnection(false);
            session.SessionStatus().ok() == Some(GattSessionStatus::Active)
        });
        for subscription in subscriptions {
            if let Some(token) = subscription.value_token {
                let _ = subscription.characteristic.RemoveValueChanged(token);
            }
            self.subscriptions.remove(&subscription.id);
        }
        if let Some(key) = connect_key {
            self.reject(key, closing.error.clone());
        }
        for procedure in procedures {
            if let Some(watchdog) = procedure.watchdog {
                watchdog.abort();
            }
            if let Some(key) = procedure.key {
                self.reject(key, closing.error.clone());
            }
        }
        if closing.emit && was_connected {
            self.emit(
                &owner,
                Event::ConnectionClosed {
                    connection_id: connection_id.clone(),
                },
            );
        }
        match closing.wait {
            Some(wait) if active => {
                let id = connection_id.clone();
                let timer = self.after(wait, move |engine| engine.finish_close(&id));
                if let Some(link) = self.connections.get_mut(connection_id) {
                    link.close_timer = Some(timer);
                }
            }
            _ => self.finish_close(connection_id),
        }
    }

    /// Forgets a closed link, answers its disconnects, and lets a waiting
    /// connect to the same device run.
    fn finish_close(&mut self, connection_id: &ConnectionId) {
        let Some(link) = self.connections.remove(connection_id) else {
            return;
        };
        if let Some(timer) = link.close_timer {
            timer.abort();
        }
        if let Some(session) = link.session {
            let [status_token, mtu_token] = link.session_tokens;
            if let Some(token) = status_token {
                let _ = session.RemoveSessionStatusChanged(token);
            }
            if let Some(token) = mtu_token {
                let _ = session.RemoveMaxPduSizeChanged(token);
            }
            let _ = session.Close();
        }
        for key in link.disconnects {
            self.resolve(key, Reply::Empty);
        }
        let waiting = self
            .device_links
            .get_mut(&link.device_id)
            .map(|device| {
                if device.connection.as_ref() == Some(connection_id) {
                    device.connection = None;
                }
                std::mem::take(&mut device.waiting)
            })
            .unwrap_or_default();
        for key in waiting {
            if self.pending(key) {
                self.connect(key, link.device_id.clone());
            }
        }
    }

    /// Closes the links of `owner` without events.
    pub(super) fn release_links(&mut self, owner: &OwnerId) {
        let targets: Vec<ConnectionId> = self
            .connections
            .iter()
            .filter(|(_, link)| &link.owner == owner)
            .map(|(connection_id, _)| connection_id.clone())
            .collect();
        for connection_id in targets {
            self.close_link(&connection_id, &Closing::silent(cancelled()));
        }
    }

    /// The radio stopped: a pending connect rejects with `error`, and each
    /// connected link ends with `connectionClosed`.
    pub(super) fn links_lost(&mut self, error: &BleError) {
        let links: Vec<(ConnectionId, Phase)> = self
            .connections
            .iter()
            .map(|(connection_id, link)| (connection_id.clone(), link.phase))
            .collect();
        for (connection_id, phase) in links {
            match phase {
                Phase::Connecting => self.fail_connect(&connection_id, error.clone()),
                Phase::Connected => {
                    self.close_link(&connection_id, &Closing::lost("Bluetooth turned off"));
                }
                Phase::Closing => self.finish_close(&connection_id),
            }
        }
    }
}

// GATT procedures

impl Engine {
    fn enqueue(&mut self, connection_id: &ConnectionId, key: u64, call: Call) {
        let token = self.token();
        let deadline_at = self
            .deadline_at(key)
            .unwrap_or_else(|| Instant::now() + PROCEDURE_DEADLINE);
        self.set_cleanup(key, Cleanup::Procedure(connection_id.clone(), token));
        if let Some(link) = self.connections.get_mut(connection_id) {
            link.queue.push(Procedure {
                token,
                key: Some(key),
                call,
                deadline_at,
                watchdog: None,
            });
        }
        self.pump(connection_id);
    }

    /// Starts queued procedures, one at a time.
    fn pump(&mut self, connection_id: &ConnectionId) {
        loop {
            let Some(link) = self
                .connections
                .get_mut(connection_id)
                .filter(|link| link.phase == Phase::Connected)
            else {
                return;
            };
            let Some(procedure) = link.queue.start_next() else {
                return;
            };
            let (token, key) = (procedure.token, procedure.key);
            if key.is_some_and(|key| !self.pending(key)) {
                self.settle(connection_id, token, Err(cancelled()));
                continue;
            }
            match self.start(connection_id, token) {
                Ok(()) => {
                    if key.is_none() {
                        self.watch(connection_id, token, PROCEDURE_DEADLINE);
                    }
                    return;
                }
                Err(error) => self.settle(connection_id, token, Err(error)),
            }
        }
    }

    /// Issues the platform call of the procedure in flight.
    fn start(&mut self, connection_id: &ConnectionId, token: u64) -> BleResult<()> {
        let closed = || disconnected("the link closed");
        let link = self.connections.get(connection_id).ok_or_else(closed)?;
        let call = link.queue.in_flight().ok_or_else(closed)?.call.clone();
        let device = link.device.clone().ok_or_else(closed)?;
        let id = connection_id.clone();
        let done = move |engine: &mut Engine, outcome: BleResult<Done>| {
            engine.procedure_done(&id, token, outcome);
        };
        match call {
            Call::Discover => {
                let operation = device
                    .GetGattServicesWithCacheModeAsync(BluetoothCacheMode::Uncached)
                    .map_err(|error| winrt_error(&error, "service discovery"))?;
                self.spawn(discover(operation), move |engine, outcome| {
                    done(engine, outcome.map(Done::Services));
                });
            }
            Call::Read(characteristic) => {
                let operation = characteristic
                    .ReadValueWithCacheModeAsync(BluetoothCacheMode::Uncached)
                    .map_err(|error| winrt_error(&error, "the read"))?;
                self.spawn(read_value(operation), move |engine, outcome| {
                    done(engine, outcome.map(Done::Bytes));
                });
            }
            Call::Write(characteristic, value, write_type) => {
                let option = match write_type {
                    WriteType::WithResponse => GattWriteOption::WriteWithResponse,
                    WriteType::WithoutResponse => GattWriteOption::WriteWithoutResponse,
                };
                let operation = buffer(&value)
                    .and_then(|value| {
                        characteristic.WriteValueWithResultAndOptionAsync(&value, option)
                    })
                    .map_err(|error| winrt_error(&error, "the write"))?;
                self.spawn(
                    write_result(operation, "the write"),
                    move |engine, outcome| {
                        done(engine, outcome.map(|()| Done::Empty));
                    },
                );
            }
            Call::Subscribe(handle, mode) => {
                let characteristic = self.listen(connection_id, &handle)?;
                let descriptor = match mode {
                    Mode::Notify => GattClientCharacteristicConfigurationDescriptorValue::Notify,
                    Mode::Indicate => {
                        GattClientCharacteristicConfigurationDescriptorValue::Indicate
                    }
                };
                let operation = characteristic
                    .WriteClientCharacteristicConfigurationDescriptorWithResultAsync(descriptor)
                    .map_err(|error| winrt_error(&error, "the subscribe"))?;
                self.spawn(
                    write_result(operation, "the subscribe"),
                    move |engine, outcome| {
                        done(engine, outcome.map(|()| Done::Empty));
                    },
                );
            }
            Call::Unsubscribe(handle) | Call::Rollback(handle) => {
                let characteristic = self
                    .connections
                    .get(connection_id)
                    .and_then(|link| link.subscriptions.get(&handle))
                    .map(|subscription| subscription.characteristic.clone())
                    .ok_or_else(|| disconnected("the subscription ended"))?;
                let operation = characteristic
                    .WriteClientCharacteristicConfigurationDescriptorWithResultAsync(
                        GattClientCharacteristicConfigurationDescriptorValue::None,
                    )
                    .map_err(|error| winrt_error(&error, "the unsubscribe"))?;
                self.spawn(
                    write_result(operation, "the unsubscribe"),
                    move |engine, outcome| {
                        done(engine, outcome.map(|()| Done::Empty));
                    },
                );
            }
        }
        Ok(())
    }

    /// Registers the value handler of a reserved subscription before its
    /// descriptor write, so that no early value is lost.
    fn listen(
        &mut self,
        connection_id: &ConnectionId,
        handle: &str,
    ) -> BleResult<GattCharacteristic> {
        let subscription = self
            .connections
            .get_mut(connection_id)
            .and_then(|link| link.subscriptions.get_mut(handle))
            .ok_or_else(|| disconnected("the subscription ended"))?;
        let poster = self.poster.clone();
        let (id, key) = (connection_id.clone(), handle.to_owned());
        let token = subscription
            .characteristic
            .ValueChanged(&handler::<GattCharacteristic, GattValueChangedEventArgs>(
                move |args| {
                    let Some(value) = args
                        .and_then(|args| args.CharacteristicValue().ok())
                        .and_then(|value| bytes(&value).ok())
                    else {
                        return;
                    };
                    let (id, key) = (id.clone(), key.clone());
                    poster.post(move |engine| engine.characteristic_changed(&id, &key, value));
                },
            ))
            .map_err(|error| winrt_error(&error, "the subscribe"))?;
        subscription.value_token = Some(token);
        Ok(subscription.characteristic.clone())
    }

    fn procedure_done(
        &mut self,
        connection_id: &ConnectionId,
        token: u64,
        outcome: BleResult<Done>,
    ) {
        let in_flight = self
            .connections
            .get(connection_id)
            .and_then(|link| link.queue.in_flight())
            .is_some_and(|procedure| procedure.token == token);
        // A late completion of a closed link or of an earlier procedure answers nothing.
        if in_flight {
            self.settle(connection_id, token, outcome);
            self.pump(connection_id);
        }
    }

    /// Ends the procedure in flight and answers its operation.
    fn settle(&mut self, connection_id: &ConnectionId, token: u64, outcome: BleResult<Done>) {
        let Some(procedure) = self
            .connections
            .get_mut(connection_id)
            .and_then(|link| link.queue.finish(|procedure| procedure.token == token))
        else {
            return;
        };
        if let Some(watchdog) = procedure.watchdog {
            watchdog.abort();
        }
        let key = procedure.key;
        let answer = match (procedure.call, outcome) {
            (Call::Subscribe(handle, _), outcome) => {
                return self.subscribe_done(connection_id, &handle, key, outcome);
            }
            (Call::Unsubscribe(handle), outcome) => {
                return self.unsubscribe_done(connection_id, &handle, key, outcome);
            }
            (Call::Rollback(handle), _) => return self.forget_subscription(connection_id, &handle),
            (_, Err(error)) => Err(error),
            (Call::Discover, Ok(Done::Services(found))) => {
                Ok(Reply::Services(self.catalog(connection_id, found)))
            }
            (Call::Read(_), Ok(Done::Bytes(value))) => Ok(Reply::Bytes {
                value_base64: BASE64.encode(value),
            }),
            (Call::Write(..), Ok(_)) => Ok(Reply::Empty),
            (Call::Discover | Call::Read(_), Ok(_)) => Err(BleError::new(
                ErrorCode::Internal,
                "the procedure returned the wrong result",
            )),
        };
        if let Some(key) = key {
            match answer {
                Ok(reply) => self.resolve(key, reply),
                Err(error) => self.reject(key, error),
            }
        }
    }

    fn subscribe_done(
        &mut self,
        connection_id: &ConnectionId,
        handle: &str,
        key: Option<u64>,
        outcome: BleResult<Done>,
    ) {
        let Some(key) = key.filter(|key| self.pending(*key)) else {
            // A cancel ended the subscribe while its descriptor write ran:
            // the rollback runs next, and the reservation holds until then.
            if outcome.is_ok() {
                let token = self.token();
                if let Some(link) = self.connections.get_mut(connection_id) {
                    link.queue.push_front(Procedure {
                        token,
                        key: None,
                        call: Call::Rollback(handle.to_owned()),
                        deadline_at: Instant::now() + PROCEDURE_DEADLINE,
                        watchdog: None,
                    });
                }
            } else {
                self.forget_subscription(connection_id, handle);
            }
            return;
        };
        if let Err(error) = outcome {
            self.forget_subscription(connection_id, handle);
            return self.reject(key, error);
        }
        let Some(link) = self.connections.get_mut(connection_id) else {
            return;
        };
        let owner = link.owner.clone();
        let Some(subscription) = link.subscriptions.get_mut(handle) else {
            return;
        };
        subscription.phase = SubscriptionPhase::Active;
        let subscription_id = subscription.id.clone();
        self.subscriptions.insert(
            subscription_id.clone(),
            SubscriptionRef {
                owner,
                connection_id: connection_id.clone(),
                handle: handle.to_owned(),
            },
        );
        self.resolve(key, Reply::SubscriptionStarted { subscription_id });
    }

    /// A failed unsubscribe keeps the subscription: the peripheral may still notify.
    fn unsubscribe_done(
        &mut self,
        connection_id: &ConnectionId,
        handle: &str,
        key: Option<u64>,
        outcome: BleResult<Done>,
    ) {
        match outcome {
            Ok(_) => {
                self.forget_subscription(connection_id, handle);
                if let Some(key) = key {
                    self.resolve(key, Reply::Empty);
                }
            }
            Err(error) => {
                if let Some(subscription) = self
                    .connections
                    .get_mut(connection_id)
                    .and_then(|link| link.subscriptions.get_mut(handle))
                {
                    subscription.phase = SubscriptionPhase::Active;
                }
                if let Some(key) = key {
                    self.reject(key, error);
                }
            }
        }
    }

    fn forget_subscription(&mut self, connection_id: &ConnectionId, handle: &str) {
        let Some(subscription) = self
            .connections
            .get_mut(connection_id)
            .and_then(|link| link.subscriptions.remove(handle))
        else {
            return;
        };
        if let Some(token) = subscription.value_token {
            let _ = subscription.characteristic.RemoveValueChanged(token);
        }
        self.subscriptions.remove(&subscription.id);
    }

    /// A deadline, `cancel` or `closeOwner` ended the operation of a procedure.
    /// A waiting procedure just leaves the queue. A timed-out procedure in
    /// flight closes the link, so its late completion never answers a later
    /// one. A cancelled one keeps its place until its completion, and the
    /// link closes when none arrives by its deadline.
    pub(super) fn procedure_aborted(
        &mut self,
        connection_id: &ConnectionId,
        token: u64,
        error: &BleError,
    ) {
        let Some(link) = self.connections.get_mut(connection_id) else {
            return;
        };
        let Some(procedure) = link
            .queue
            .in_flight()
            .filter(|procedure| procedure.token == token)
        else {
            for procedure in link
                .queue
                .remove_waiting(|procedure| procedure.token == token)
            {
                match procedure.call {
                    Call::Subscribe(handle, _) => {
                        link.subscriptions.remove(&handle);
                    }
                    Call::Unsubscribe(handle) => {
                        if let Some(subscription) = link.subscriptions.get_mut(&handle) {
                            subscription.phase = SubscriptionPhase::Active;
                        }
                    }
                    _ => {}
                }
            }
            return;
        };
        if error.code == ErrorCode::Timeout {
            return self.close_link(
                connection_id,
                &Closing::lost("a GATT procedure passed its deadline"),
            );
        }
        let delay = procedure
            .deadline_at
            .saturating_duration_since(Instant::now());
        self.watch(connection_id, token, delay);
    }

    /// Closes the link when the procedure is still in flight after `delay`.
    fn watch(&mut self, connection_id: &ConnectionId, token: u64, delay: Duration) {
        let id = connection_id.clone();
        let watchdog = self.after(delay, move |engine| {
            let stuck = engine
                .connections
                .get(&id)
                .and_then(|link| link.queue.in_flight())
                .is_some_and(|procedure| procedure.token == token);
            if stuck {
                engine.close_link(&id, &Closing::lost("a GATT procedure passed its deadline"));
            }
        });
        if let Some(procedure) = self
            .connections
            .get_mut(connection_id)
            .and_then(|link| link.queue.in_flight_mut())
            .filter(|procedure| procedure.token == token)
        {
            if let Some(previous) = procedure.watchdog.replace(watchdog) {
                previous.abort();
            }
        }
    }

    /// The discovered services with stable handles. Refreshes the handle table.
    fn catalog(
        &mut self,
        connection_id: &ConnectionId,
        found: Vec<(GattDeviceService, Vec<GattCharacteristic>)>,
    ) -> Vec<ServiceInstance> {
        let Some(link) = self.connections.get_mut(connection_id) else {
            return Vec::new();
        };
        link.handles.forget_attributes();
        let mut instances = Vec::with_capacity(found.len());
        for (service, characteristics) in found {
            let service_uuid = service.Uuid().map(uuid).unwrap_or_default();
            let service_handle = link.handles.service((
                service.AttributeHandle().unwrap_or_default(),
                service_uuid.clone(),
            ));
            let characteristics = characteristics
                .into_iter()
                .map(|characteristic| {
                    let characteristic_uuid = characteristic.Uuid().map(uuid).unwrap_or_default();
                    let properties = properties(&characteristic);
                    let attribute = (
                        characteristic.AttributeHandle().unwrap_or_default(),
                        characteristic_uuid.clone(),
                    );
                    CharacteristicInstance {
                        handle: CharacteristicHandle::new(
                            link.handles.characteristic(attribute, characteristic),
                        ),
                        uuid: characteristic_uuid,
                        properties,
                    }
                })
                .collect();
            link.services.push(service);
            instances.push(ServiceInstance {
                handle: ServiceHandle::new(service_handle),
                uuid: service_uuid,
                characteristics,
            });
        }
        instances
    }

    fn characteristic_changed(
        &mut self,
        connection_id: &ConnectionId,
        handle: &str,
        value: Vec<u8>,
    ) {
        let Some(link) = self
            .connections
            .get(connection_id)
            .filter(|link| link.phase == Phase::Connected)
        else {
            return;
        };
        let Some(subscription) = link
            .subscriptions
            .get(handle)
            .filter(|subscription| subscription.phase == SubscriptionPhase::Active)
        else {
            return;
        };
        self.emit(
            &link.owner,
            Event::CharacteristicValue {
                subscription_id: subscription.id.clone(),
                value_base64: BASE64.encode(value),
            },
        );
    }
}

fn properties(characteristic: &GattCharacteristic) -> crate::CharacteristicProperties {
    characteristic
        .CharacteristicProperties()
        .map(|properties| properties_from_bits(properties.0))
        .unwrap_or_default()
}

fn check_status(status: GattCommunicationStatus, att: Option<u8>, action: &str) -> BleResult<()> {
    if status == GattCommunicationStatus::Success {
        Ok(())
    } else {
        Err(communication_error(status.0, att, action))
    }
}

/// Discovers every service, then the characteristics of each, from the remote.
async fn discover(
    operation: IAsyncOperation<GattDeviceServicesResult>,
) -> BleResult<Vec<(GattDeviceService, Vec<GattCharacteristic>)>> {
    let failed = |error: windows::core::Error| winrt_error(&error, "service discovery");
    let result = operation.await.map_err(failed)?;
    check_status(
        result.Status().map_err(failed)?,
        att_error(result.ProtocolError()),
        "service discovery",
    )?;
    let services = result
        .Services()
        .and_then(|services| items(&services))
        .map_err(failed)?;
    let mut discovered = Vec::with_capacity(services.len());
    for service in services {
        let result = service
            .GetCharacteristicsWithCacheModeAsync(BluetoothCacheMode::Uncached)
            .map_err(failed)?
            .await
            .map_err(failed)?;
        let status = result.Status().map_err(failed)?;
        // Windows keeps some services, such as HID, to itself: they come without characteristics.
        let characteristics = if status == GattCommunicationStatus::AccessDenied {
            Vec::new()
        } else {
            check_status(
                status,
                att_error(result.ProtocolError()),
                "service discovery",
            )?;
            result
                .Characteristics()
                .and_then(|characteristics| items(&characteristics))
                .map_err(failed)?
        };
        discovered.push((service, characteristics));
    }
    Ok(discovered)
}

async fn read_value(operation: IAsyncOperation<GattReadResult>) -> BleResult<Vec<u8>> {
    let failed = |error: windows::core::Error| winrt_error(&error, "the read");
    let result = operation.await.map_err(failed)?;
    check_status(
        result.Status().map_err(failed)?,
        att_error(result.ProtocolError()),
        "the read",
    )?;
    result
        .Value()
        .and_then(|value| bytes(&value))
        .map_err(failed)
}

async fn write_result(
    operation: IAsyncOperation<GattWriteResult>,
    action: &'static str,
) -> BleResult<()> {
    let result = operation
        .await
        .map_err(|error| winrt_error(&error, action))?;
    check_status(
        result
            .Status()
            .map_err(|error| winrt_error(&error, action))?,
        att_error(result.ProtocolError()),
        action,
    )
}
