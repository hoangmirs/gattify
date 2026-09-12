//! Peer scenarios over two mock backends linked by [`MockAir`].
//!
//! Side 0 hosts and side 1 joins. The tests run in paused tokio time, so the
//! 5 s ACK deadline and the 10 s dial timeout pass instantly.

use std::{sync::Arc, time::Duration};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use tokio::{sync::mpsc, time::Instant};

use crate::{
    mock::MockAir,
    peer::{
        peer_label, peer_owner, CloseReason, EndpointOptions, Frame, FrameKind, PeerDriver,
        PeerEmitter, PeerEvent, RX_CHARACTERISTIC_UUID, TX_CHARACTERISTIC_UUID,
    },
    BleResult, BleRuntime, Command, ConnectOptions, DeliveryOutcome, ErrorCode, Event, EventSink,
    OwnerId, PeerId, Reply, ResourceSnapshot, WriteType,
};

const SERVICE: &str = "80ff87c3-8e84-4914-aedc-0d6a3ba5534d";
const LABEL: &str = "main";
const HOST: usize = 0;
const JOINER: usize = 1;

struct Node {
    runtime: BleRuntime,
    driver: PeerDriver,
    events: mpsc::UnboundedReceiver<PeerEvent>,
}

impl Node {
    fn start(runtime: BleRuntime, inbox: mpsc::UnboundedReceiver<(OwnerId, Event)>) -> Self {
        let (events_tx, events) = mpsc::unbounded_channel();
        let emit: PeerEmitter = Arc::new(move |label, event| {
            assert_eq!(label, LABEL);
            let _ = events_tx.send(event);
        });
        let driver = PeerDriver::new(runtime.clone(), emit);
        tokio::spawn(driver.clone().run(inbox));
        Self {
            runtime,
            driver,
            events,
        }
    }

    async fn next_event(&mut self) -> PeerEvent {
        tokio::time::timeout(Duration::from_secs(60), self.events.recv())
            .await
            .expect("no peer event within 60 s")
            .expect("the driver stopped")
    }

    /// Lets every task settle, then reports whether another event is waiting.
    async fn is_quiet(&mut self) -> bool {
        tokio::time::sleep(Duration::from_millis(50)).await;
        self.events.try_recv().is_err()
    }

    async fn resources(&self) -> ResourceSnapshot {
        let reply = self
            .runtime
            .execute(peer_owner(LABEL), Command::DebugResources, None)
            .await
            .unwrap();
        let Reply::Resources(resources) = reply else {
            panic!("expected resources");
        };
        resources
    }
}

/// Routes peer driver events to the driver, and everything else nowhere.
fn sink() -> (EventSink, mpsc::UnboundedReceiver<(OwnerId, Event)>) {
    let (inbox, inbox_rx) = mpsc::unbounded_channel();
    let sink: EventSink = Arc::new(move |owner: OwnerId, event| {
        if peer_label(&owner).is_some() {
            let _ = inbox.send((owner, event));
        }
    });
    (sink, inbox_rx)
}

fn pair() -> (MockAir, Node, Node) {
    let (host_sink, host_inbox) = sink();
    let (joiner_sink, joiner_inbox) = sink();
    let (air, host, joiner) = MockAir::link(host_sink, joiner_sink);
    (
        air,
        Node::start(BleRuntime::new(host), host_inbox),
        Node::start(BleRuntime::new(joiner), joiner_inbox),
    )
}

fn options(listen: bool) -> EndpointOptions {
    EndpointOptions {
        service_uuid: SERVICE.into(),
        local_name: Some("Host".into()),
        max_logical_payload: 16 * 1024,
        listen,
    }
}

struct Connected {
    air: MockAir,
    host: Node,
    joiner: Node,
    host_endpoint: String,
    host_peer: PeerId,
    joiner_peer: PeerId,
}

async fn connected() -> Connected {
    let (air, mut host, joiner) = pair();
    let host_endpoint = host
        .driver
        .create_endpoint(LABEL, options(true))
        .await
        .unwrap();
    let joiner_endpoint = joiner
        .driver
        .create_endpoint(LABEL, options(false))
        .await
        .unwrap();
    let joiner_peer = joiner
        .driver
        .dial(LABEL, &joiner_endpoint, MockAir::device_id(HOST))
        .await
        .unwrap();
    let PeerEvent::Ready {
        endpoint_id,
        peer_id: host_peer,
    } = host.next_event().await
    else {
        panic!("expected the host peer");
    };
    assert_eq!(endpoint_id, host_endpoint);
    Connected {
        air,
        host,
        joiner,
        host_endpoint,
        host_peer,
        joiner_peer,
    }
}

async fn send(node: &Node, peer: &PeerId, bytes: &[u8]) -> BleResult<crate::peer::SendReceipt> {
    node.driver
        .send(LABEL, peer, bytes.to_vec(), Duration::from_secs(30))
        .await
}

fn message(peer_id: &PeerId, bytes: &[u8]) -> PeerEvent {
    PeerEvent::Message {
        peer_id: peer_id.clone(),
        bytes: bytes.to_vec(),
    }
}

fn closed(peer_id: &PeerId, reason: CloseReason) -> PeerEvent {
    PeerEvent::Closed {
        peer_id: peer_id.clone(),
        reason,
    }
}

#[tokio::test(start_paused = true)]
async fn a_joiner_dials_a_listening_host() {
    let mut connected = connected().await;
    assert!(connected.host.is_quiet().await);
    assert_eq!(connected.joiner.resources().await.connections, 1);
    assert_eq!(connected.host.resources().await.servers, 1);
}

#[tokio::test(start_paused = true)]
async fn a_full_message_crosses_in_each_direction() {
    let mut connected = connected().await;
    let payload: Vec<u8> = (0..16 * 1024_usize)
        .map(|index| u8::try_from(index % 251).unwrap())
        .collect();

    let receipt = send(&connected.joiner, &connected.joiner_peer, &payload)
        .await
        .unwrap();
    assert_eq!(receipt.delivery, DeliveryOutcome::TransportAcknowledged);
    assert_eq!(
        connected.host.next_event().await,
        message(&connected.host_peer, &payload)
    );

    let reply: Vec<u8> = payload.iter().rev().copied().collect();
    let receipt = send(&connected.host, &connected.host_peer, &reply)
        .await
        .unwrap();
    assert_eq!(receipt.delivery, DeliveryOutcome::TransportAcknowledged);
    assert_eq!(
        connected.joiner.next_event().await,
        message(&connected.joiner_peer, &reply)
    );
}

#[tokio::test(start_paused = true)]
async fn a_dropped_frame_is_sent_again() {
    let mut connected = connected().await;
    connected.air.drop_next_frame(JOINER);
    let started = Instant::now();

    let receipt = send(&connected.joiner, &connected.joiner_peer, b"hi")
        .await
        .unwrap();

    assert_eq!(receipt.delivery, DeliveryOutcome::TransportAcknowledged);
    assert!(started.elapsed() >= Duration::from_secs(5));
    assert_eq!(
        connected.host.next_event().await,
        message(&connected.host_peer, b"hi")
    );
    assert!(connected.host.is_quiet().await);
}

#[tokio::test(start_paused = true)]
async fn a_duplicate_after_a_lost_ack_is_delivered_once() {
    let mut connected = connected().await;
    connected.air.drop_next_frame(HOST);

    let receipt = send(&connected.joiner, &connected.joiner_peer, b"once")
        .await
        .unwrap();

    assert_eq!(receipt.delivery, DeliveryOutcome::TransportAcknowledged);
    assert_eq!(
        connected.host.next_event().await,
        message(&connected.host_peer, b"once")
    );
    assert!(connected.host.is_quiet().await);
}

#[tokio::test(start_paused = true)]
async fn a_joiner_close_reaches_the_host() {
    let mut connected = connected().await;
    connected
        .joiner
        .driver
        .close_peer(LABEL, &connected.joiner_peer)
        .unwrap();

    assert_eq!(
        connected.joiner.next_event().await,
        closed(&connected.joiner_peer, CloseReason::Local)
    );
    assert_eq!(
        connected.host.next_event().await,
        closed(&connected.host_peer, CloseReason::Remote)
    );
    assert!(connected.host.is_quiet().await);
    assert_eq!(connected.joiner.resources().await.connections, 0);
}

#[tokio::test(start_paused = true)]
async fn a_host_close_reaches_the_joiner() {
    let mut connected = connected().await;
    connected
        .host
        .driver
        .close_peer(LABEL, &connected.host_peer)
        .unwrap();

    assert_eq!(
        connected.host.next_event().await,
        closed(&connected.host_peer, CloseReason::Local)
    );
    assert_eq!(
        connected.joiner.next_event().await,
        closed(&connected.joiner_peer, CloseReason::Remote)
    );
    assert!(connected.joiner.is_quiet().await);
    assert_eq!(connected.joiner.resources().await.connections, 0);
}

#[tokio::test(start_paused = true)]
async fn a_lost_link_closes_both_peers_and_fails_the_send() {
    let mut connected = connected().await;
    connected.air.drop_next_frame(JOINER);
    let joiner = Arc::new(connected.joiner.driver.clone());
    let peer = connected.joiner_peer.clone();
    let pending = tokio::spawn(async move {
        joiner
            .send(LABEL, &peer, b"lost".to_vec(), Duration::from_secs(30))
            .await
    });
    tokio::time::sleep(Duration::from_millis(10)).await;

    connected.air.disconnect();

    let error = pending.await.unwrap().unwrap_err();
    assert_eq!(error.code, ErrorCode::Disconnected);
    assert_eq!(error.delivery, Some(DeliveryOutcome::Unknown));
    assert_eq!(
        connected.joiner.next_event().await,
        closed(&connected.joiner_peer, CloseReason::Lost)
    );
    assert_eq!(
        connected.host.next_event().await,
        closed(&connected.host_peer, CloseReason::Lost)
    );
}

#[tokio::test(start_paused = true)]
async fn a_dial_without_hello_ack_times_out() {
    let (air, mut host, joiner) = pair();
    host.driver
        .create_endpoint(LABEL, options(true))
        .await
        .unwrap();
    let endpoint = joiner
        .driver
        .create_endpoint(LABEL, options(false))
        .await
        .unwrap();
    air.drop_next_frame(HOST);
    let started = Instant::now();

    let error = joiner
        .driver
        .dial(LABEL, &endpoint, MockAir::device_id(HOST))
        .await
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::Timeout);
    assert!(started.elapsed() >= Duration::from_secs(10));
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(joiner.resources().await.connections, 0);
    let PeerEvent::Ready { peer_id, .. } = host.next_event().await else {
        panic!("expected the host peer");
    };
    assert_eq!(host.next_event().await, closed(&peer_id, CloseReason::Lost));
}

#[tokio::test(start_paused = true)]
async fn a_second_hello_from_one_central_replaces_its_peer() {
    let (_air, mut host, joiner) = pair();
    host.driver
        .create_endpoint(LABEL, options(true))
        .await
        .unwrap();
    // A raw client that speaks the handshake by hand, twice.
    let raw = OwnerId::new("webview:raw");
    let run = |command| joiner.runtime.execute(raw.clone(), command, None);
    let Reply::Connected { connection_id, .. } = run(Command::Connect {
        device_id: MockAir::device_id(HOST),
        options: ConnectOptions { timeout_ms: None },
    })
    .await
    .unwrap() else {
        panic!("expected a connection");
    };
    let Reply::Services(services) = run(Command::DiscoverServices {
        connection_id: connection_id.clone(),
    })
    .await
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
    run(Command::Subscribe {
        connection_id: connection_id.clone(),
        characteristic: handle(TX_CHARACTERISTIC_UUID),
    })
    .await
    .unwrap();
    let hello = BASE64.encode(
        Frame {
            kind: FrameKind::Hello,
            message_id: 0,
            fragment_index: 0,
            fragment_count: 1,
            total_length: 0,
            payload: Vec::new(),
        }
        .encode(0)
        .unwrap(),
    );
    let write_hello = || Command::Write {
        connection_id: connection_id.clone(),
        characteristic: handle(RX_CHARACTERISTIC_UUID),
        value_base64: hello.clone(),
        write_type: WriteType::WithResponse,
    };

    run(write_hello()).await.unwrap();
    let PeerEvent::Ready { peer_id: first, .. } = host.next_event().await else {
        panic!("expected the first peer");
    };
    run(write_hello()).await.unwrap();

    assert_eq!(host.next_event().await, closed(&first, CloseReason::Lost));
    let PeerEvent::Ready {
        peer_id: second, ..
    } = host.next_event().await
    else {
        panic!("expected the second peer");
    };
    assert_ne!(first, second);
}

#[tokio::test(start_paused = true)]
async fn a_peer_belongs_to_its_webview() {
    let connected = connected().await;
    let error = connected
        .joiner
        .driver
        .send(
            "other",
            &connected.joiner_peer,
            b"x".to_vec(),
            Duration::from_secs(1),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidHandle);
    let error = connected
        .joiner
        .driver
        .close_peer("other", &connected.joiner_peer)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidHandle);
}

#[tokio::test(start_paused = true)]
async fn closing_the_host_endpoint_releases_its_server() {
    let mut connected = connected().await;
    connected
        .host
        .driver
        .close_endpoint(LABEL, &connected.host_endpoint)
        .await
        .unwrap();

    assert_eq!(
        connected.host.next_event().await,
        closed(&connected.host_peer, CloseReason::Local)
    );
    assert_eq!(
        connected.joiner.next_event().await,
        closed(&connected.joiner_peer, CloseReason::Remote)
    );
    assert_eq!(connected.host.resources().await.servers, 0);
}

#[tokio::test(start_paused = true)]
async fn a_reloaded_webview_forgets_its_peers_silently() {
    let mut connected = connected().await;
    connected.air.drop_next_frame(JOINER);
    let joiner = Arc::new(connected.joiner.driver.clone());
    let peer = connected.joiner_peer.clone();
    let pending = tokio::spawn(async move {
        joiner
            .send(LABEL, &peer, b"gone".to_vec(), Duration::from_secs(30))
            .await
    });
    tokio::time::sleep(Duration::from_millis(10)).await;

    connected.joiner.driver.forget_label(LABEL);

    assert_eq!(
        pending.await.unwrap().unwrap_err().code,
        ErrorCode::Disconnected
    );
    assert!(connected.joiner.is_quiet().await);
}

#[tokio::test(start_paused = true)]
async fn an_endpoint_outside_the_limits_is_rejected() {
    let (_air, host, _joiner) = pair();
    let error = host
        .driver
        .create_endpoint(
            LABEL,
            EndpointOptions {
                max_logical_payload: 16 * 1024 + 1,
                ..options(true)
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidArgument);
    assert_eq!(host.resources().await.servers, 0);
}
