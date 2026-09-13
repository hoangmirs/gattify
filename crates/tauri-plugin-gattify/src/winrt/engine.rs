use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    future::Future,
    sync::{atomic::AtomicU64, Arc},
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use tokio::{
    sync::{mpsc, oneshot},
    task::AbortHandle,
};

use super::{
    adapter::AdapterSlot,
    central::{Connection, DeviceLink, SubscriptionRef},
    ids::{IdAllocator, Remotes},
    peripheral::{AdvertisementRecord, ReadyWrite, Server},
    queue::InOrder,
    scan::Sightings,
    scanner::{ScanRecord, WatcherRecord},
    status::{capabilities, deadline_for, permissions, readiness_error, StateTracker},
};
use crate::{
    AdapterState, BleError, BleResult, Command, ConnectionId, ErrorCode, Event, EventSink,
    OperationContext, OperationId, OwnerId, Reply, ResourceSnapshot, ScanId, ServerId,
    SubscriptionId,
};

pub(super) type Job = Box<dyn FnOnce(&mut Engine) + Send>;

/// Posts jobs to the engine thread. `WinRT` callbacks hold one, so posting
/// never blocks and never takes a lock that a callback could need.
#[derive(Clone)]
pub(super) struct Poster(pub(super) mpsc::UnboundedSender<Job>);

impl Poster {
    /// Returns false when the engine thread is gone.
    pub(super) fn post(&self, job: impl FnOnce(&mut Engine) + Send + 'static) -> bool {
        self.0.send(Box::new(job)).is_ok()
    }
}

/// What an operation started, which stops when a deadline, `cancel` or
/// `closeOwner` ends the operation early.
pub(super) enum Cleanup {
    None,
    Connect(ConnectionId),
    Procedure(ConnectionId, u64),
    CreateServer(ServerId),
    Advertise(ServerId),
    Notify(ServerId, u64),
}

/// One `execute` call. It answers exactly once.
struct Operation {
    id: OperationId,
    owner: OwnerId,
    reply: oneshot::Sender<BleResult<Reply>>,
    deadline_at: Option<Instant>,
    timer: Option<AbortHandle>,
    cleanup: Cleanup,
}

/// The state of the backend. Only the engine thread touches it.
pub(super) struct Engine {
    pub(super) poster: Poster,
    sink: EventSink,
    pub(super) ids: IdAllocator,
    next_key: u64,
    next_token: u64,
    operations: HashMap<u64, Operation>,
    keys: HashMap<OperationId, u64>,
    pub(super) adapter: AdapterSlot,
    pub(super) adapter_states: StateTracker,
    pub(super) devices: Remotes<u64>,
    pub(super) device_links: HashMap<String, DeviceLink>,
    pub(super) sightings: Sightings,
    pub(super) scans: BTreeMap<ScanId, ScanRecord>,
    pub(super) watcher: Option<WatcherRecord>,
    pub(super) connections: BTreeMap<ConnectionId, Connection>,
    pub(super) subscriptions: HashMap<SubscriptionId, SubscriptionRef>,
    pub(super) centrals: Remotes<String>,
    pub(super) servers: BTreeMap<ServerId, Server>,
    pub(super) advertisement: Option<AdvertisementRecord>,
    pub(super) writes: InOrder<ReadyWrite>,
    pub(super) write_sequence: Arc<AtomicU64>,
}

impl Engine {
    pub(super) fn new(poster: Poster, sink: EventSink) -> Self {
        Self {
            poster,
            sink,
            ids: IdAllocator::default(),
            next_key: 0,
            next_token: 0,
            operations: HashMap::new(),
            keys: HashMap::new(),
            adapter: AdapterSlot::Unloaded,
            adapter_states: StateTracker::default(),
            devices: Remotes::new("device"),
            device_links: HashMap::new(),
            sightings: Sightings::default(),
            scans: BTreeMap::new(),
            watcher: None,
            connections: BTreeMap::new(),
            subscriptions: HashMap::new(),
            centrals: Remotes::new("central"),
            servers: BTreeMap::new(),
            advertisement: None,
            writes: InOrder::default(),
            write_sequence: Arc::new(AtomicU64::new(0)),
        }
    }

    pub(super) async fn run(mut self, mut inbox: mpsc::UnboundedReceiver<Job>) {
        while let Some(job) = inbox.recv().await {
            job(&mut self);
        }
    }

    pub(super) fn execute(
        &mut self,
        context: OperationContext,
        command: Command,
        reply: oneshot::Sender<BleResult<Reply>>,
    ) {
        let key = self.next_key;
        self.next_key += 1;
        let deadline = deadline_for(&command, context.deadline_millis);
        let timer =
            deadline.map(|delay| self.after(delay, move |engine| engine.abort(key, &timeout())));
        self.keys.insert(context.operation_id.clone(), key);
        self.operations.insert(
            key,
            Operation {
                id: context.operation_id,
                owner: context.owner_id,
                reply,
                deadline_at: deadline.map(|delay| Instant::now() + delay),
                timer,
                cleanup: Cleanup::None,
            },
        );
        self.dispatch(key, command);
    }

    fn dispatch(&mut self, key: u64, command: Command) {
        match command {
            Command::GetState => self.when_adapter(key, false, |engine, key| {
                let state = engine.adapter_state();
                engine.resolve(key, Reply::State(state));
            }),
            Command::GetCapabilities => self.when_adapter(key, false, |engine, key| {
                let facts = engine.adapter_facts();
                engine.resolve(key, Reply::Capabilities(capabilities(facts.as_ref())));
            }),
            Command::CheckPermissions | Command::RequestPermissions(_) => {
                self.resolve(key, Reply::Permissions(permissions()));
            }
            Command::StartScan(options) => self.start_scan(key, &options),
            Command::StopScan { scan_id } => self.stop_scan(key, &scan_id),
            Command::Connect { device_id, .. } => self.connect(key, device_id.0),
            Command::Disconnect { connection_id } => self.disconnect(key, &connection_id),
            Command::DiscoverServices { connection_id } => {
                self.discover_services(key, &connection_id);
            }
            Command::Read {
                connection_id,
                characteristic,
            } => self.read(key, &connection_id, characteristic.as_str()),
            Command::Write {
                connection_id,
                characteristic,
                value_base64,
                write_type,
            } => self.write(
                key,
                &connection_id,
                characteristic.as_str(),
                &value_base64,
                write_type,
            ),
            Command::Subscribe {
                connection_id,
                characteristic,
            } => self.subscribe(key, &connection_id, characteristic.as_str()),
            Command::Unsubscribe { subscription_id } => self.unsubscribe(key, &subscription_id),
            Command::CreateServer(definition) => self.create_server(key, definition),
            Command::CloseServer { server_id } => self.close_server(key, &server_id),
            Command::StartAdvertising { server_id, options } => {
                self.start_advertising(key, &server_id, &options);
            }
            Command::StopAdvertising { server_id } => self.stop_advertising(key, &server_id),
            Command::SetValue {
                server_id,
                characteristic_key,
                value_base64,
            } => self.set_value(key, &server_id, &characteristic_key, &value_base64),
            Command::Notify {
                server_id,
                peer_id,
                characteristic_key,
                value_base64,
            } => self.notify(key, &server_id, peer_id, characteristic_key, &value_base64),
            Command::Cancel { operation_id } => self.cancel(key, &operation_id),
            Command::CloseOwner => self.close_owner(key),
            Command::DebugResources => self.debug_resources(key),
        }
    }

    pub(super) fn pending(&self, key: u64) -> bool {
        self.operations.contains_key(&key)
    }

    pub(super) fn owner(&self, key: u64) -> Option<OwnerId> {
        self.operations
            .get(&key)
            .map(|operation| operation.owner.clone())
    }

    pub(super) fn deadline_at(&self, key: u64) -> Option<Instant> {
        self.operations
            .get(&key)
            .and_then(|operation| operation.deadline_at)
    }

    pub(super) fn set_cleanup(&mut self, key: u64, cleanup: Cleanup) {
        if let Some(operation) = self.operations.get_mut(&key) {
            operation.cleanup = cleanup;
        }
    }

    fn finish(&mut self, key: u64, result: BleResult<Reply>) -> Option<Cleanup> {
        let operation = self.operations.remove(&key)?;
        if self.keys.get(&operation.id) == Some(&key) {
            self.keys.remove(&operation.id);
        }
        if let Some(timer) = operation.timer {
            timer.abort();
        }
        let _ = operation.reply.send(result);
        Some(operation.cleanup)
    }

    pub(super) fn resolve(&mut self, key: u64, reply: Reply) {
        self.finish(key, Ok(reply));
    }

    pub(super) fn reject(&mut self, key: u64, error: BleError) {
        self.finish(key, Err(error));
    }

    /// Rejects the operation, then stops what it started.
    pub(super) fn abort(&mut self, key: u64, error: &BleError) {
        let Some(cleanup) = self.finish(key, Err(error.clone())) else {
            return;
        };
        match cleanup {
            Cleanup::None => {}
            Cleanup::Connect(connection_id) => self.connect_aborted(&connection_id),
            Cleanup::Procedure(connection_id, token) => {
                self.procedure_aborted(&connection_id, token, error);
            }
            Cleanup::CreateServer(server_id) => self.registration_aborted(&server_id),
            Cleanup::Advertise(server_id) => self.advertising_aborted(&server_id),
            Cleanup::Notify(server_id, token) => self.notification_aborted(&server_id, token),
        }
    }

    pub(super) fn emit(&self, owner: &OwnerId, event: Event) {
        (self.sink)(owner.clone(), event);
    }

    /// Awaits `future` on the engine runtime, then runs `then` on the engine with its output.
    pub(super) fn spawn<T: Send + 'static>(
        &self,
        future: impl Future<Output = T> + Send + 'static,
        then: impl FnOnce(&mut Engine, T) + Send + 'static,
    ) {
        let poster = self.poster.clone();
        tokio::spawn(async move {
            let output = future.await;
            poster.post(move |engine| then(engine, output));
        });
    }

    /// Runs `then` on the engine after `delay`, unless the handle aborts it first.
    pub(super) fn after(
        &self,
        delay: Duration,
        then: impl FnOnce(&mut Engine) + Send + 'static,
    ) -> AbortHandle {
        let poster = self.poster.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            poster.post(then);
        })
        .abort_handle()
    }

    /// A number that matches a platform call with its completion.
    pub(super) fn token(&mut self) -> u64 {
        self.next_token += 1;
        self.next_token
    }

    fn cancel(&mut self, key: u64, target: &OperationId) {
        let owner = self.owner(key);
        if let Some(&target_key) = self.keys.get(target) {
            let same_owner = self
                .operations
                .get(&target_key)
                .is_some_and(|operation| Some(&operation.owner) == owner.as_ref());
            if target_key != key && same_owner {
                self.abort(target_key, &cancelled());
            }
        }
        self.resolve(key, Reply::Empty);
    }

    /// Releases everything the owner holds, without events.
    fn close_owner(&mut self, key: u64) {
        let Some(owner) = self.owner(key) else {
            return;
        };
        let pending: Vec<u64> = self
            .operations
            .iter()
            .filter(|(other, operation)| **other != key && operation.owner == owner)
            .map(|(other, _)| *other)
            .collect();
        for other in pending {
            self.abort(other, &cancelled());
        }
        self.release_scans(&owner);
        self.release_links(&owner);
        self.release_servers(&owner);
        self.resolve(key, Reply::Empty);
    }

    fn debug_resources(&mut self, key: u64) {
        let Some(owner) = self.owner(key) else {
            return;
        };
        let snapshot = ResourceSnapshot {
            scans: self
                .scans
                .values()
                .filter(|scan| scan.owner == owner)
                .count(),
            connections: self
                .connections
                .values()
                .filter(|link| link.owner == owner && link.is_connected())
                .count(),
            subscriptions: self
                .subscriptions
                .values()
                .filter(|subscription| subscription.owner == owner)
                .count(),
            servers: self
                .servers
                .values()
                .filter(|server| server.owner == owner && !server.is_registering())
                .count(),
        };
        self.resolve(key, Reply::Resources(snapshot));
    }

    fn resource_owners(&self) -> BTreeSet<OwnerId> {
        let mut owners: BTreeSet<OwnerId> =
            self.scans.values().map(|scan| scan.owner.clone()).collect();
        owners.extend(
            self.connections
                .values()
                .filter(|link| !link.is_closing())
                .map(|link| link.owner.clone()),
        );
        owners.extend(
            self.subscriptions
                .values()
                .map(|subscription| subscription.owner.clone()),
        );
        owners.extend(self.servers.values().map(|server| server.owner.clone()));
        owners
    }

    /// Tells every owner that holds a resource about a new adapter state.
    /// When the adapter is no longer on, every scan, link and server ends.
    pub(super) fn adapter_changed(&mut self, state: AdapterState) {
        for owner in self.resource_owners() {
            self.emit(&owner, Event::AdapterStateChanged { state });
        }
        if let Some(error) = readiness_error(state) {
            self.scans_lost();
            self.links_lost(&error);
            self.servers_lost(&error);
        }
    }
}

pub(super) fn timeout() -> BleError {
    BleError::new(ErrorCode::Timeout, "the deadline passed")
}

pub(super) fn cancelled() -> BleError {
    BleError::new(ErrorCode::Cancelled, "the operation was cancelled")
}

pub(super) fn disconnected(message: &str) -> BleError {
    BleError::new(ErrorCode::Disconnected, message)
}

pub(super) fn invalid_argument(message: impl Into<String>) -> BleError {
    BleError::new(ErrorCode::InvalidArgument, message)
}

pub(super) fn busy(message: impl Into<String>) -> BleError {
    BleError::new(ErrorCode::Busy, message)
}

pub(super) fn decode(value_base64: &str) -> BleResult<Vec<u8>> {
    BASE64
        .decode(value_base64)
        .map_err(|_| invalid_argument("valueBase64 is not valid base64"))
}
