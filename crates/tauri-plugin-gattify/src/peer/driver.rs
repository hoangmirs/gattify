//! Runs the peer protocol over raw GATT commands and events.
//!
//! A host listens: it registers the peer service, advertises it, and answers
//! each HELLO from a central with a new peer. A joiner dials: it connects,
//! checks the Info value, subscribes to TX and sends HELLO. Both sides then
//! move complete messages with one stop-and-wait [`Sender`] and one
//! [`Receiver`] per peer.
//!
//! The driver runs its commands under the owner `gattify-peer:<label>`, where
//! the label names the webview that asked for the peer.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::{
    sync::{mpsc, oneshot, Notify},
    time::Instant,
};

use crate::{
    normalize_uuid,
    peer::{
        Frame, FrameKind, ReceiveAction, Receiver, ReceiverLimits, SendAction, SendLimits, Sender,
        INFO_CHARACTERISTIC_UUID, PROTOCOL_MAJOR, RX_CHARACTERISTIC_UUID, TX_CHARACTERISTIC_UUID,
    },
    AdvertisingOptions, BleError, BleResult, BleRuntime, CharacteristicHandle,
    CharacteristicProperties, Command, ConnectOptions, ConnectionId, DeliveryOutcome, DeviceId,
    ErrorCode, Event, LinkLimits, LocalCharacteristic, LocalService, OwnerId, PeerId, Reply,
    ResourceId, ServerDefinition, ServerId, ServiceInstance, SubscriptionId, WriteType,
};

/// The frame size when the native layer reports none: the ATT minimum.
const DEFAULT_VALUE_LIMIT: u32 = 20;
const DIAL_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(100);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
/// The largest logical message the v1 defaults allow.
pub const MAX_LOGICAL_PAYLOAD: usize = 16 * 1024;
/// The longest value one ATT attribute holds.
const ATTRIBUTE_VALUE_MAX: u32 = 512;
const PEER_OWNER_PREFIX: &str = "gattify-peer:";
const SERVICE_KEY: &str = "peer";
const RX_KEY: &str = "peer/rx";
const TX_KEY: &str = "peer/tx";

/// Options of `create_endpoint`, as the TypeScript API sends them.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EndpointOptions {
    pub service_uuid: String,
    #[serde(default)]
    pub local_name: Option<String>,
    #[serde(default = "default_max_logical_payload")]
    pub max_logical_payload: usize,
    /// A host listens: it registers the service and advertises it.
    #[serde(default)]
    pub listen: bool,
}

fn default_max_logical_payload() -> usize {
    MAX_LOGICAL_PAYLOAD
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CloseReason {
    /// This side closed the peer.
    Local,
    /// The other side sent CLOSE.
    Remote,
    /// The link went away without CLOSE.
    Lost,
}

/// An event for the webview that owns a peer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PeerEvent {
    /// A peer can move messages. `dialed` is true for a peer this side dialed,
    /// false for a peer that dialed this host.
    Ready {
        endpoint_id: String,
        peer_id: PeerId,
        dialed: bool,
    },
    Message {
        peer_id: PeerId,
        bytes: Vec<u8>,
    },
    Closed {
        peer_id: PeerId,
        reason: CloseReason,
    },
}

impl PeerEvent {
    /// The event name after the `gattify://` prefix.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Ready { .. } => "peer-ready",
            Self::Message { .. } => "peer-message",
            Self::Closed { .. } => "peer-closed",
        }
    }

    /// The payload that the TypeScript API reads.
    #[must_use]
    pub fn payload(&self) -> serde_json::Value {
        match self {
            Self::Ready {
                endpoint_id,
                peer_id,
                dialed,
            } => json!({ "endpointId": endpoint_id, "peerId": peer_id, "dialed": dialed }),
            Self::Message { peer_id, bytes } => {
                json!({ "peerId": peer_id, "valueBase64": BASE64.encode(bytes) })
            }
            Self::Closed { peer_id, reason } => json!({ "peerId": peer_id, "reason": reason }),
        }
    }
}

/// Delivers a [`PeerEvent`] to the webview with the given label.
pub type PeerEmitter = Arc<dyn Fn(&str, PeerEvent) + Send + Sync>;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendReceipt {
    pub message_id: u32,
    pub delivery: DeliveryOutcome,
}

/// The owner under which the driver acts for the webview `label`.
#[must_use]
pub fn peer_owner(label: &str) -> OwnerId {
    OwnerId::new(format!("{PEER_OWNER_PREFIX}{label}"))
}

/// The webview label of a peer driver owner.
#[must_use]
pub fn peer_label(owner: &OwnerId) -> Option<&str> {
    owner.as_str().strip_prefix(PEER_OWNER_PREFIX)
}

#[derive(Clone, Debug)]
enum Link {
    /// A joiner writes frames to the RX characteristic of the host.
    Central {
        connection_id: ConnectionId,
        rx: CharacteristicHandle,
        subscription_id: SubscriptionId,
    },
    /// A host notifies frames on TX to one central.
    Peripheral {
        server_id: ServerId,
        central: PeerId,
    },
}

impl Link {
    fn frame_command(&self, frame: &[u8]) -> Command {
        let value_base64 = BASE64.encode(frame);
        match self {
            Self::Central {
                connection_id, rx, ..
            } => Command::Write {
                connection_id: connection_id.clone(),
                characteristic: rx.clone(),
                value_base64,
                write_type: WriteType::WithResponse,
            },
            Self::Peripheral { server_id, central } => Command::Notify {
                server_id: server_id.clone(),
                peer_id: central.clone(),
                characteristic_key: TX_KEY.into(),
                value_base64,
            },
        }
    }
}

enum Outbound {
    Frame {
        bytes: Vec<u8>,
        written: Option<oneshot::Sender<BleResult<()>>>,
    },
    /// Ends the writer of a closed peer.
    Finish {
        send_close: bool,
        disconnect: bool,
        done: Option<oneshot::Sender<()>>,
    },
}

struct Endpoint {
    label: String,
    service_uuid: String,
    max_logical_payload: usize,
    server_id: Option<ServerId>,
    /// The notification size of each subscribed central.
    notification_sizes: HashMap<PeerId, usize>,
    /// The peer of each central that sent HELLO.
    centrals: HashMap<PeerId, PeerId>,
}

struct Peer {
    label: String,
    endpoint_id: String,
    link: Link,
    max_logical_payload: usize,
    /// A joiner peer is ready after `HELLO_ACK`. A host peer is ready at once.
    ready: bool,
    hello_ack: Option<oneshot::Sender<()>>,
    sender: Sender,
    receiver: Receiver,
    waiters: HashMap<u32, oneshot::Sender<BleResult<SendReceipt>>>,
    outbound: mpsc::UnboundedSender<Outbound>,
    closing: Arc<AtomicBool>,
    wake: Arc<Notify>,
}

#[derive(Default)]
struct State {
    endpoints: HashMap<String, Endpoint>,
    peers: HashMap<PeerId, Peer>,
}

/// Work that must wait until the state lock is released.
#[derive(Default)]
struct Effects {
    events: Vec<(String, PeerEvent)>,
    receipts: Vec<(
        oneshot::Sender<BleResult<SendReceipt>>,
        BleResult<SendReceipt>,
    )>,
}

impl Effects {
    fn apply(self, emit: &PeerEmitter) {
        for (waiter, receipt) in self.receipts {
            let _ = waiter.send(receipt);
        }
        for (label, event) in self.events {
            emit(&label, event);
        }
    }
}

struct Closing {
    reason: CloseReason,
    send_close: bool,
    disconnect: bool,
    announce: bool,
    done: Option<oneshot::Sender<()>>,
}

struct Shared {
    runtime: BleRuntime,
    emit: PeerEmitter,
    state: Mutex<State>,
    next_id: AtomicU64,
    started: Instant,
}

impl Shared {
    fn now_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn next_id(&self, prefix: &str) -> String {
        format!("{prefix}-{}", self.next_id.fetch_add(1, Ordering::Relaxed))
    }
}

/// Runs endpoints and peers for every webview of an app.
#[derive(Clone)]
pub struct PeerDriver {
    shared: Arc<Shared>,
}

impl PeerDriver {
    #[must_use]
    pub fn new(runtime: BleRuntime, emit: PeerEmitter) -> Self {
        Self {
            shared: Arc::new(Shared {
                runtime,
                emit,
                state: Mutex::new(State::default()),
                next_id: AtomicU64::new(1),
                started: Instant::now(),
            }),
        }
    }

    /// Handles the backend events of every peer driver owner until `inbox` closes.
    pub async fn run(self, mut inbox: mpsc::UnboundedReceiver<(OwnerId, Event)>) {
        while let Some((owner, event)) = inbox.recv().await {
            self.handle_event(&owner, event);
        }
    }

    /// Registers an endpoint. A listening endpoint registers the peer service
    /// and advertises it.
    ///
    /// # Errors
    ///
    /// Returns an invalid-argument error for bad options, or the error of the
    /// command that failed.
    pub async fn create_endpoint(
        &self,
        label: &str,
        options: EndpointOptions,
    ) -> BleResult<String> {
        if options.max_logical_payload == 0 || options.max_logical_payload > MAX_LOGICAL_PAYLOAD {
            return Err(BleError::new(
                ErrorCode::InvalidArgument,
                format!("maxLogicalPayload must be between 1 and {MAX_LOGICAL_PAYLOAD} bytes"),
            ));
        }
        let service_uuid = normalize_uuid(&options.service_uuid).ok_or_else(|| {
            BleError::new(ErrorCode::InvalidArgument, "serviceUuid is not a UUID")
        })?;
        let owner = peer_owner(label);
        let endpoint_id = self.shared.next_id("endpoint");
        let server_id = if options.listen {
            let reply = self
                .execute(
                    &owner,
                    Command::CreateServer(server_definition(&service_uuid)),
                )
                .await?;
            let Reply::ServerCreated { server_id } = reply else {
                return Err(unexpected(&reply));
            };
            Some(server_id)
        } else {
            None
        };
        self.shared.state.lock().endpoints.insert(
            endpoint_id.clone(),
            Endpoint {
                label: label.to_owned(),
                service_uuid: service_uuid.clone(),
                max_logical_payload: options.max_logical_payload,
                server_id: server_id.clone(),
                notification_sizes: HashMap::new(),
                centrals: HashMap::new(),
            },
        );
        if let Some(server_id) = server_id {
            let advertising = self
                .execute(
                    &owner,
                    Command::StartAdvertising {
                        server_id: server_id.clone(),
                        options: AdvertisingOptions {
                            service_uuid,
                            local_name: options.local_name,
                            local_name_optional: true,
                        },
                    },
                )
                .await;
            if let Err(error) = advertising {
                self.shared.state.lock().endpoints.remove(&endpoint_id);
                let _ = self
                    .execute(&owner, Command::CloseServer { server_id })
                    .await;
                return Err(error);
            }
        }
        Ok(endpoint_id)
    }

    /// Connects to a host and runs the handshake.
    ///
    /// # Errors
    ///
    /// Returns an invalid-handle error for an unknown endpoint, a protocol
    /// error when the device is not a v1 host, a timeout when no `HELLO_ACK`
    /// arrives within 10 s, or the error of the command that failed.
    pub async fn dial(
        &self,
        label: &str,
        endpoint_id: &str,
        device_id: DeviceId,
    ) -> BleResult<PeerId> {
        let (service_uuid, max_logical_payload) = {
            let state = self.shared.state.lock();
            let endpoint = state
                .endpoints
                .get(endpoint_id)
                .filter(|endpoint| endpoint.label == label)
                .ok_or_else(|| BleError::invalid_handle(ResourceId::new(endpoint_id)))?;
            (endpoint.service_uuid.clone(), endpoint.max_logical_payload)
        };
        let owner = peer_owner(label);
        let reply = self
            .execute(
                &owner,
                Command::Connect {
                    device_id,
                    options: ConnectOptions { timeout_ms: None },
                },
            )
            .await?;
        let Reply::Connected {
            connection_id,
            limits,
        } = reply
        else {
            return Err(unexpected(&reply));
        };
        let peer_id = PeerId::new(self.shared.next_id("peer"));
        let handshake = Handshake {
            label,
            endpoint_id,
            owner: &owner,
            peer_id: &peer_id,
            connection_id: &connection_id,
            limits: &limits,
            service_uuid: &service_uuid,
            max_logical_payload,
        };
        if let Err(error) = self.handshake(handshake).await {
            let mut effects = Effects::default();
            let registered = {
                let mut state = self.shared.state.lock();
                let registered = state.peers.contains_key(&peer_id);
                // A peer that announced itself also announces its end.
                remove_peer(
                    &mut state,
                    &peer_id,
                    Closing {
                        reason: CloseReason::Lost,
                        send_close: false,
                        disconnect: true,
                        announce: true,
                        done: None,
                    },
                    &mut effects,
                );
                registered
            };
            effects.apply(&self.shared.emit);
            if !registered {
                let _ = self
                    .execute(&owner, Command::Disconnect { connection_id })
                    .await;
            }
            return Err(error);
        }
        Ok(peer_id)
    }

    async fn handshake(&self, handshake: Handshake<'_>) -> BleResult<()> {
        let Handshake {
            label,
            endpoint_id,
            owner,
            peer_id,
            connection_id,
            limits,
            service_uuid,
            max_logical_payload,
        } = handshake;
        let reply = self
            .execute(
                owner,
                Command::DiscoverServices {
                    connection_id: connection_id.clone(),
                },
            )
            .await?;
        let Reply::Services(services) = reply else {
            return Err(unexpected(&reply));
        };
        let [info, rx, tx] = peer_characteristics(&services, service_uuid)?;

        let reply = self
            .execute(
                owner,
                Command::Read {
                    connection_id: connection_id.clone(),
                    characteristic: info,
                },
            )
            .await?;
        let Reply::Bytes { value_base64 } = reply else {
            return Err(unexpected(&reply));
        };
        check_version(&value_base64)?;

        let reply = self
            .execute(
                owner,
                Command::Subscribe {
                    connection_id: connection_id.clone(),
                    characteristic: tx,
                },
            )
            .await?;
        let Reply::SubscriptionStarted { subscription_id } = reply else {
            return Err(unexpected(&reply));
        };

        let value_limit = frame_size(limits.write_with_response);
        let (hello_ack, acknowledged) = oneshot::channel();
        let (written_tx, written) = oneshot::channel();
        {
            let mut state = self.shared.state.lock();
            if !state.endpoints.contains_key(endpoint_id) {
                return Err(BleError::invalid_handle(ResourceId::new(endpoint_id)));
            }
            let peer = self.insert_peer(
                &mut state,
                NewPeer {
                    label,
                    endpoint_id,
                    peer_id: peer_id.clone(),
                    link: Link::Central {
                        connection_id: connection_id.clone(),
                        rx,
                        subscription_id,
                    },
                    value_limit,
                    max_logical_payload,
                    hello_ack: Some(hello_ack),
                },
            );
            let _ = peer.outbound.send(Outbound::Frame {
                bytes: control_frame(FrameKind::Hello),
                written: Some(written_tx),
            });
        }
        // A HELLO_ACK proves that the host received HELLO even when the write
        // response was lost, so a failed write still waits for it.
        let write_error = written.await.unwrap_or_else(|_| Err(link_closed())).err();
        match tokio::time::timeout(DIAL_TIMEOUT, acknowledged).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => Err(write_error.unwrap_or_else(link_closed)),
            Err(_) => Err(write_error.unwrap_or_else(|| {
                BleError::new(ErrorCode::Timeout, "the host sent no HELLO_ACK within 10 s")
            })),
        }
    }

    /// Sends one complete message and waits for its ACK.
    ///
    /// # Errors
    ///
    /// Returns an invalid-handle error for an unknown peer, a payload or queue
    /// error for a message that cannot be admitted, or a failure that carries
    /// its [`DeliveryOutcome`].
    pub async fn send(
        &self,
        label: &str,
        peer_id: &PeerId,
        bytes: Vec<u8>,
        timeout: Duration,
    ) -> BleResult<SendReceipt> {
        let receipt = {
            let mut state = self.shared.state.lock();
            let peer = state
                .peers
                .get_mut(peer_id)
                .filter(|peer| peer.label == label && peer.ready)
                .ok_or_else(|| BleError::invalid_handle(ResourceId::new(peer_id.as_str())))?;
            let message_id = peer
                .sender
                .enqueue(bytes)
                .map_err(|error| with_delivery(error, DeliveryOutcome::NotSubmitted))?;
            let (waiter, receipt) = oneshot::channel();
            peer.waiters.insert(message_id, waiter);
            peer.wake.notify_one();
            receipt
        };
        match tokio::time::timeout(timeout, receipt).await {
            Ok(Ok(receipt)) => receipt,
            Ok(Err(_)) => Err(with_delivery(link_closed(), DeliveryOutcome::Unknown)),
            Err(_) => Err(with_delivery(
                BleError::new(
                    ErrorCode::Timeout,
                    "no ACK arrived before the send timeout; delivery may have occurred",
                ),
                DeliveryOutcome::Unknown,
            )),
        }
    }

    /// Sends CLOSE without waiting for it, and forgets the peer. A joiner
    /// then disconnects.
    ///
    /// # Errors
    ///
    /// Returns an invalid-handle error for an unknown peer.
    pub fn close_peer(&self, label: &str, peer_id: &PeerId) -> BleResult<()> {
        let mut effects = Effects::default();
        {
            let mut state = self.shared.state.lock();
            let peer = state
                .peers
                .get(peer_id)
                .filter(|peer| peer.label == label && peer.ready)
                .ok_or_else(|| BleError::invalid_handle(ResourceId::new(peer_id.as_str())))?;
            let disconnect = matches!(peer.link, Link::Central { .. });
            remove_peer(
                &mut state,
                peer_id,
                Closing {
                    reason: CloseReason::Local,
                    send_close: true,
                    disconnect,
                    announce: true,
                    done: None,
                },
                &mut effects,
            );
        }
        effects.apply(&self.shared.emit);
        Ok(())
    }

    /// Closes every peer of the endpoint, then stops its advertisement and
    /// closes its server.
    ///
    /// # Errors
    ///
    /// Returns an invalid-handle error for an unknown endpoint.
    pub async fn close_endpoint(&self, label: &str, endpoint_id: &str) -> BleResult<()> {
        let mut effects = Effects::default();
        let mut finished = Vec::new();
        let server_id = {
            let mut state = self.shared.state.lock();
            let endpoint = state
                .endpoints
                .get(endpoint_id)
                .filter(|endpoint| endpoint.label == label)
                .ok_or_else(|| BleError::invalid_handle(ResourceId::new(endpoint_id)))?;
            let server_id = endpoint.server_id.clone();
            let peers: Vec<_> = state
                .peers
                .iter()
                .filter(|(_, peer)| peer.endpoint_id == endpoint_id)
                .map(|(peer_id, peer)| (peer_id.clone(), peer.ready, peer.link.clone()))
                .collect();
            for (peer_id, ready, link) in peers {
                let (done, finish) = oneshot::channel();
                finished.push(finish);
                remove_peer(
                    &mut state,
                    &peer_id,
                    Closing {
                        reason: CloseReason::Local,
                        send_close: ready,
                        disconnect: matches!(link, Link::Central { .. }),
                        announce: true,
                        done: Some(done),
                    },
                    &mut effects,
                );
            }
            state.endpoints.remove(endpoint_id);
            server_id
        };
        effects.apply(&self.shared.emit);
        // The CLOSE notifications must leave before the server goes away.
        for finish in finished {
            let _ = tokio::time::timeout(CLOSE_TIMEOUT, finish).await;
        }
        if let Some(server_id) = server_id {
            let owner = peer_owner(label);
            let _ = self
                .execute(
                    &owner,
                    Command::StopAdvertising {
                        server_id: server_id.clone(),
                    },
                )
                .await;
            let _ = self
                .execute(&owner, Command::CloseServer { server_id })
                .await;
        }
        Ok(())
    }

    /// Forgets every endpoint and peer of a webview that reloaded or closed.
    ///
    /// It emits nothing: the page that owned them is gone. The caller releases
    /// the native resources with `CloseOwner`.
    pub fn forget_label(&self, label: &str) {
        let mut effects = Effects::default();
        {
            let mut state = self.shared.state.lock();
            let peers: Vec<_> = state
                .peers
                .iter()
                .filter(|(_, peer)| peer.label == label)
                .map(|(peer_id, _)| peer_id.clone())
                .collect();
            for peer_id in peers {
                remove_peer(
                    &mut state,
                    &peer_id,
                    Closing {
                        reason: CloseReason::Lost,
                        send_close: false,
                        disconnect: false,
                        announce: false,
                        done: None,
                    },
                    &mut effects,
                );
            }
            state
                .endpoints
                .retain(|_, endpoint| endpoint.label != label);
        }
        effects.apply(&self.shared.emit);
    }

    /// Applies one backend event of a peer driver owner.
    pub fn handle_event(&self, owner: &OwnerId, event: Event) {
        let Some(label) = peer_label(owner) else {
            return;
        };
        let mut effects = Effects::default();
        {
            let mut state = self.shared.state.lock();
            match event {
                Event::SubscriptionChanged {
                    server_id,
                    peer_id,
                    characteristic_key,
                    subscribed,
                    max_value_length,
                } if characteristic_key == TX_KEY => {
                    on_subscription(
                        &mut state,
                        label,
                        &server_id,
                        peer_id,
                        subscribed.then(|| frame_size(max_value_length)),
                        &mut effects,
                    );
                }
                Event::ServerWrite {
                    server_id,
                    peer_id: Some(central),
                    characteristic_key,
                    value_base64,
                } if characteristic_key == RX_KEY => {
                    if let Ok(bytes) = BASE64.decode(value_base64) {
                        self.on_host_frame(
                            &mut state,
                            label,
                            &server_id,
                            central,
                            &bytes,
                            &mut effects,
                        );
                    }
                }
                Event::CharacteristicValue {
                    subscription_id,
                    value_base64,
                } => {
                    if let Ok(bytes) = BASE64.decode(value_base64) {
                        self.on_joiner_frame(
                            &mut state,
                            label,
                            &subscription_id,
                            &bytes,
                            &mut effects,
                        );
                    }
                }
                Event::ConnectionClosed { connection_id } => {
                    lose_peers(
                        &mut state,
                        label,
                        |peer| matches!(&peer.link, Link::Central { connection_id: id, .. } if *id == connection_id),
                        &mut effects,
                    );
                }
                Event::CriticalStateLoss { resource_id, .. } => {
                    lose_peers(
                        &mut state,
                        label,
                        |peer| match &peer.link {
                            Link::Central { connection_id, .. } => {
                                connection_id.as_str() == resource_id
                            }
                            Link::Peripheral { server_id, .. } => server_id.as_str() == resource_id,
                        },
                        &mut effects,
                    );
                }
                _ => {}
            }
        }
        effects.apply(&self.shared.emit);
    }

    fn on_host_frame(
        &self,
        state: &mut State,
        label: &str,
        server_id: &ServerId,
        central: PeerId,
        bytes: &[u8],
        effects: &mut Effects,
    ) {
        let Some((endpoint_id, endpoint)) = state.endpoints.iter().find(|(_, endpoint)| {
            endpoint.label == label && endpoint.server_id.as_ref() == Some(server_id)
        }) else {
            return;
        };
        let endpoint_id = endpoint_id.clone();
        let Ok(frame) = Frame::decode(bytes, endpoint.max_logical_payload) else {
            return;
        };
        if frame.kind != FrameKind::Hello {
            if let Some(peer_id) = endpoint.centrals.get(&central).cloned() {
                on_peer_frame(
                    state,
                    &peer_id,
                    &frame,
                    bytes,
                    self.shared.now_ms(),
                    effects,
                );
            }
            return;
        }

        let value_limit = endpoint
            .notification_sizes
            .get(&central)
            .copied()
            .unwrap_or_else(|| frame_size(None));
        let max_logical_payload = endpoint.max_logical_payload;
        let previous = state
            .endpoints
            .get_mut(&endpoint_id)
            .and_then(|endpoint| endpoint.centrals.remove(&central));
        if let Some(previous) = previous {
            remove_peer(
                state,
                &previous,
                Closing {
                    reason: CloseReason::Lost,
                    send_close: false,
                    disconnect: false,
                    announce: true,
                    done: None,
                },
                effects,
            );
        }
        let peer_id = PeerId::new(self.shared.next_id("peer"));
        let peer = self.insert_peer(
            state,
            NewPeer {
                label,
                endpoint_id: &endpoint_id,
                peer_id: peer_id.clone(),
                link: Link::Peripheral {
                    server_id: server_id.clone(),
                    central: central.clone(),
                },
                value_limit,
                max_logical_payload,
                hello_ack: None,
            },
        );
        let _ = peer.outbound.send(Outbound::Frame {
            bytes: control_frame(FrameKind::HelloAck),
            written: None,
        });
        if let Some(endpoint) = state.endpoints.get_mut(&endpoint_id) {
            endpoint.centrals.insert(central, peer_id.clone());
        }
        effects.events.push((
            label.to_owned(),
            PeerEvent::Ready {
                endpoint_id,
                peer_id,
                dialed: false,
            },
        ));
    }

    fn on_joiner_frame(
        &self,
        state: &mut State,
        label: &str,
        subscription_id: &SubscriptionId,
        bytes: &[u8],
        effects: &mut Effects,
    ) {
        let Some((peer_id, peer)) = state.peers.iter_mut().find(|(_, peer)| {
            peer.label == label
                && matches!(&peer.link, Link::Central { subscription_id: id, .. } if id == subscription_id)
        }) else {
            return;
        };
        let Ok(frame) = Frame::decode(bytes, peer.max_logical_payload) else {
            return;
        };
        match frame.kind {
            FrameKind::HelloAck => {
                if !peer.ready {
                    peer.ready = true;
                    if let Some(hello_ack) = peer.hello_ack.take() {
                        let _ = hello_ack.send(());
                    }
                    peer.wake.notify_one();
                    // Announced here, on the event path, so the webview learns
                    // the peer before any message that follows HELLO_ACK.
                    effects.events.push((
                        peer.label.clone(),
                        PeerEvent::Ready {
                            endpoint_id: peer.endpoint_id.clone(),
                            peer_id: peer_id.clone(),
                            dialed: true,
                        },
                    ));
                }
            }
            FrameKind::Hello => {}
            _ if peer.ready => {
                let peer_id = peer_id.clone();
                on_peer_frame(
                    state,
                    &peer_id,
                    &frame,
                    bytes,
                    self.shared.now_ms(),
                    effects,
                );
            }
            _ => {}
        }
    }

    fn insert_peer<'a>(&self, state: &'a mut State, new: NewPeer<'_>) -> &'a mut Peer {
        let (outbound, outbound_rx) = mpsc::unbounded_channel();
        let closing = Arc::new(AtomicBool::new(false));
        let wake = Arc::new(Notify::new());
        let peer_id = new.peer_id;
        state.peers.insert(
            peer_id.clone(),
            Peer {
                label: new.label.to_owned(),
                endpoint_id: new.endpoint_id.to_owned(),
                link: new.link.clone(),
                max_logical_payload: new.max_logical_payload,
                ready: new.hello_ack.is_none(),
                hello_ack: new.hello_ack,
                sender: Sender::new(SendLimits {
                    max_logical_size: new.max_logical_payload,
                    value_limit: new.value_limit,
                    ..SendLimits::default()
                }),
                receiver: Receiver::new(ReceiverLimits {
                    max_logical_size: new.max_logical_payload,
                    ..ReceiverLimits::default()
                }),
                waiters: HashMap::new(),
                outbound,
                closing: closing.clone(),
                wake: wake.clone(),
            },
        );
        // Both tasks start after the peer exists, so neither sees it missing.
        tokio::spawn(write_frames(
            self.shared.runtime.clone(),
            peer_owner(new.label),
            new.link,
            outbound_rx,
            closing,
        ));
        tokio::spawn(pump(self.shared.clone(), peer_id.clone(), wake));
        state
            .peers
            .get_mut(&peer_id)
            .expect("the peer was inserted above")
    }

    async fn execute(&self, owner: &OwnerId, command: Command) -> BleResult<Reply> {
        self.shared
            .runtime
            .execute(owner.clone(), command, None)
            .await
    }
}

struct Handshake<'a> {
    label: &'a str,
    endpoint_id: &'a str,
    owner: &'a OwnerId,
    peer_id: &'a PeerId,
    connection_id: &'a ConnectionId,
    limits: &'a LinkLimits,
    service_uuid: &'a str,
    max_logical_payload: usize,
}

struct NewPeer<'a> {
    label: &'a str,
    endpoint_id: &'a str,
    peer_id: PeerId,
    link: Link,
    value_limit: usize,
    max_logical_payload: usize,
    /// Present for a joiner, which becomes ready on `HELLO_ACK`.
    hello_ack: Option<oneshot::Sender<()>>,
}

/// Records the notification size of a central, or loses its peer when it
/// unsubscribes. `size` is `None` for an unsubscription.
fn on_subscription(
    state: &mut State,
    label: &str,
    server_id: &ServerId,
    central: PeerId,
    size: Option<usize>,
    effects: &mut Effects,
) {
    let Some(endpoint) = state
        .endpoints
        .values_mut()
        .find(|endpoint| endpoint.label == label && endpoint.server_id.as_ref() == Some(server_id))
    else {
        return;
    };
    if let Some(size) = size {
        endpoint.notification_sizes.insert(central, size);
        return;
    }
    endpoint.notification_sizes.remove(&central);
    if let Some(peer_id) = endpoint.centrals.remove(&central) {
        remove_peer(
            state,
            &peer_id,
            Closing {
                reason: CloseReason::Lost,
                send_close: false,
                disconnect: false,
                announce: true,
                done: None,
            },
            effects,
        );
    }
}

/// Applies a DATA, ACK or CLOSE frame to a ready peer.
fn on_peer_frame(
    state: &mut State,
    peer_id: &PeerId,
    frame: &Frame,
    bytes: &[u8],
    now_ms: u64,
    effects: &mut Effects,
) {
    let Some(peer) = state.peers.get_mut(peer_id) else {
        return;
    };
    match frame.kind {
        FrameKind::Data => match peer.receiver.receive_at(bytes, now_ms) {
            Ok(ReceiveAction::Message { payload, ack, .. }) => {
                // The message goes to the webview at once; the queue only bounds admission.
                let _ = peer.receiver.pop_message();
                let _ = peer.outbound.send(Outbound::Frame {
                    bytes: ack,
                    written: None,
                });
                effects.events.push((
                    peer.label.clone(),
                    PeerEvent::Message {
                        peer_id: peer_id.clone(),
                        bytes: payload,
                    },
                ));
            }
            Ok(ReceiveAction::DuplicateAck { ack, .. }) => {
                let _ = peer.outbound.send(Outbound::Frame {
                    bytes: ack,
                    written: None,
                });
            }
            // A partial message waits for its other fragments. A rejected
            // frame gets no ACK, so the sender retransmits it.
            Ok(_) | Err(_) => {}
        },
        FrameKind::Ack => {
            if let Ok(SendAction::Acknowledged { message_id }) =
                peer.sender.acknowledge(frame.message_id)
            {
                if let Some(waiter) = peer.waiters.remove(&message_id) {
                    effects.receipts.push((
                        waiter,
                        Ok(SendReceipt {
                            message_id,
                            delivery: DeliveryOutcome::TransportAcknowledged,
                        }),
                    ));
                }
                peer.wake.notify_one();
            }
        }
        FrameKind::Close => {
            let disconnect = matches!(peer.link, Link::Central { .. });
            remove_peer(
                state,
                peer_id,
                Closing {
                    reason: CloseReason::Remote,
                    send_close: false,
                    disconnect,
                    announce: true,
                    done: None,
                },
                effects,
            );
        }
        FrameKind::Hello | FrameKind::HelloAck => {}
    }
}

fn lose_peers(state: &mut State, label: &str, lost: impl Fn(&Peer) -> bool, effects: &mut Effects) {
    let peers: Vec<_> = state
        .peers
        .iter()
        .filter(|(_, peer)| peer.label == label && lost(peer))
        .map(|(peer_id, _)| peer_id.clone())
        .collect();
    for peer_id in peers {
        remove_peer(
            state,
            &peer_id,
            Closing {
                reason: CloseReason::Lost,
                send_close: false,
                disconnect: false,
                announce: true,
                done: None,
            },
            effects,
        );
    }
}

/// Removes a peer, fails its pending sends and ends its writer.
fn remove_peer(state: &mut State, peer_id: &PeerId, close: Closing, effects: &mut Effects) {
    let Some(mut peer) = state.peers.remove(peer_id) else {
        return;
    };
    if let Link::Peripheral { central, .. } = &peer.link {
        if let Some(endpoint) = state.endpoints.get_mut(&peer.endpoint_id) {
            if endpoint.centrals.get(central) == Some(peer_id) {
                endpoint.centrals.remove(central);
            }
        }
    }
    peer.closing.store(true, Ordering::Release);
    let _ = peer.outbound.send(Outbound::Finish {
        send_close: close.send_close,
        disconnect: close.disconnect,
        done: close.done,
    });
    peer.wake.notify_one();

    let in_flight = peer.sender.in_flight();
    peer.sender.disconnect();
    for (message_id, waiter) in peer.waiters.drain() {
        let delivery = if Some(message_id) == in_flight {
            DeliveryOutcome::Unknown
        } else {
            DeliveryOutcome::NotSubmitted
        };
        effects.receipts.push((
            waiter,
            Err(with_delivery(
                BleError::new(ErrorCode::Disconnected, "the peer closed"),
                delivery,
            )),
        ));
    }
    if close.announce && peer.ready {
        effects.events.push((
            peer.label,
            PeerEvent::Closed {
                peer_id: peer_id.clone(),
                reason: close.reason,
            },
        ));
    }
}

/// Writes the frames of one peer in order.
async fn write_frames(
    runtime: BleRuntime,
    owner: OwnerId,
    link: Link,
    mut outbound: mpsc::UnboundedReceiver<Outbound>,
    closing: Arc<AtomicBool>,
) {
    while let Some(item) = outbound.recv().await {
        match item {
            Outbound::Frame { bytes, written } => {
                let result = if closing.load(Ordering::Acquire) {
                    Err(link_closed())
                } else {
                    runtime
                        .execute(owner.clone(), link.frame_command(&bytes), None)
                        .await
                        .map(|_| ())
                };
                if let Some(written) = written {
                    let _ = written.send(result);
                }
            }
            Outbound::Finish {
                send_close,
                disconnect,
                done,
            } => {
                if send_close {
                    let close = link.frame_command(&control_frame(FrameKind::Close));
                    let _ = tokio::time::timeout(
                        CLOSE_TIMEOUT,
                        runtime.execute(owner.clone(), close, None),
                    )
                    .await;
                }
                if let (true, Link::Central { connection_id, .. }) = (disconnect, &link) {
                    let _ = runtime
                        .execute(
                            owner.clone(),
                            Command::Disconnect {
                                connection_id: connection_id.clone(),
                            },
                            None,
                        )
                        .await;
                }
                if let Some(done) = done {
                    let _ = done.send(());
                }
                return;
            }
        }
    }
}

enum Step {
    Written(u32, oneshot::Receiver<BleResult<()>>),
    Effects(Effects),
    Wait { busy: bool },
}

/// Moves the messages of one peer through its [`Sender`].
async fn pump(shared: Arc<Shared>, peer_id: PeerId, wake: Arc<Notify>) {
    loop {
        let step = {
            let mut state = shared.state.lock();
            let Some(peer) = state.peers.get_mut(&peer_id) else {
                return;
            };
            if peer.ready {
                match peer.sender.poll(shared.now_ms()) {
                    SendAction::Submit {
                        message_id, frames, ..
                    } => {
                        let (written_tx, written) = oneshot::channel();
                        let mut written_tx = Some(written_tx);
                        let last = frames.len().saturating_sub(1);
                        for (index, bytes) in frames.into_iter().enumerate() {
                            let written = if index == last {
                                written_tx.take()
                            } else {
                                None
                            };
                            let _ = peer.outbound.send(Outbound::Frame { bytes, written });
                        }
                        Step::Written(message_id, written)
                    }
                    SendAction::Failed { message_id, error } => {
                        let mut effects = Effects::default();
                        if let Some(waiter) = peer.waiters.remove(&message_id) {
                            let error = if error.delivery.is_some() {
                                error
                            } else {
                                with_delivery(error, DeliveryOutcome::NotSubmitted)
                            };
                            effects.receipts.push((waiter, Err(error)));
                        }
                        Step::Effects(effects)
                    }
                    SendAction::Acknowledged { .. } => Step::Effects(Effects::default()),
                    SendAction::Idle => Step::Wait {
                        busy: peer.sender.has_work(),
                    },
                }
            } else {
                Step::Wait { busy: false }
            }
        };
        match step {
            Step::Written(message_id, written) => {
                let _ = written.await;
                let now_ms = shared.now_ms();
                if let Some(peer) = shared.state.lock().peers.get_mut(&peer_id) {
                    peer.sender.mark_submitted(message_id, now_ms);
                }
            }
            Step::Effects(effects) => effects.apply(&shared.emit),
            Step::Wait { busy: true } => {
                let _ = tokio::time::timeout(POLL_INTERVAL, wake.notified()).await;
            }
            Step::Wait { busy: false } => wake.notified().await,
        }
    }
}

/// The Info, RX and TX handles of the endpoint service, in that order.
fn peer_characteristics(
    services: &[ServiceInstance],
    service_uuid: &str,
) -> BleResult<[CharacteristicHandle; 3]> {
    let service = services
        .iter()
        .find(|service| normalize_uuid(&service.uuid).as_deref() == Some(service_uuid))
        .ok_or_else(|| mismatch("the device does not offer the endpoint service"))?;
    let characteristic = |uuid: &str| {
        service
            .characteristics
            .iter()
            .find(|characteristic| normalize_uuid(&characteristic.uuid).as_deref() == Some(uuid))
            .map(|characteristic| characteristic.handle.clone())
            .ok_or_else(|| mismatch("the endpoint service lacks a peer characteristic"))
    };
    Ok([
        characteristic(INFO_CHARACTERISTIC_UUID)?,
        characteristic(RX_CHARACTERISTIC_UUID)?,
        characteristic(TX_CHARACTERISTIC_UUID)?,
    ])
}

/// Requires the Info value of a v1 host: the protocol major version.
fn check_version(value_base64: &str) -> BleResult<()> {
    let version = BASE64
        .decode(value_base64)
        .map_err(|_| mismatch("the Info value is not base64"))?;
    if version == [PROTOCOL_MAJOR] {
        Ok(())
    } else {
        Err(mismatch(format!(
            "the host speaks protocol version {version:?}, not {PROTOCOL_MAJOR}"
        )))
    }
}

fn server_definition(service_uuid: &str) -> ServerDefinition {
    let characteristic =
        |key: &str, uuid: &str, properties, initial: Option<String>, max| LocalCharacteristic {
            instance_key: key.into(),
            uuid: uuid.into(),
            properties,
            initial_value_base64: initial,
            max_value_length: max,
        };
    ServerDefinition {
        services: vec![LocalService {
            instance_key: SERVICE_KEY.into(),
            uuid: service_uuid.into(),
            primary: true,
            characteristics: vec![
                characteristic(
                    "info",
                    INFO_CHARACTERISTIC_UUID,
                    CharacteristicProperties {
                        read: true,
                        ..CharacteristicProperties::default()
                    },
                    Some(BASE64.encode([PROTOCOL_MAJOR])),
                    1,
                ),
                characteristic(
                    "rx",
                    RX_CHARACTERISTIC_UUID,
                    CharacteristicProperties {
                        write: true,
                        ..CharacteristicProperties::default()
                    },
                    None,
                    ATTRIBUTE_VALUE_MAX,
                ),
                characteristic(
                    "tx",
                    TX_CHARACTERISTIC_UUID,
                    CharacteristicProperties {
                        notify: true,
                        ..CharacteristicProperties::default()
                    },
                    None,
                    ATTRIBUTE_VALUE_MAX,
                ),
            ],
        }],
    }
}

/// The frame size for a reported value length: 20 bytes when unknown, and
/// never more than one attribute holds.
fn frame_size(reported: Option<u32>) -> usize {
    reported
        .unwrap_or(DEFAULT_VALUE_LIMIT)
        .min(ATTRIBUTE_VALUE_MAX) as usize
}

/// A header-only HELLO, `HELLO_ACK` or CLOSE frame.
fn control_frame(kind: FrameKind) -> Vec<u8> {
    Frame {
        kind,
        message_id: 0,
        fragment_index: 0,
        fragment_count: 1,
        total_length: 0,
        payload: Vec::new(),
    }
    .encode(0)
    .expect("a header-only control frame is always valid")
}

fn mismatch(message: impl Into<String>) -> BleError {
    BleError::new(ErrorCode::ProtocolMismatch, message)
}

fn link_closed() -> BleError {
    BleError::new(ErrorCode::Disconnected, "the link closed")
}

fn unexpected(reply: &Reply) -> BleError {
    BleError::new(
        ErrorCode::Internal,
        format!("the backend answered with an unexpected reply: {reply:?}"),
    )
}

fn with_delivery(mut error: BleError, delivery: DeliveryOutcome) -> BleError {
    error.delivery = Some(delivery);
    error
}
