use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    AdapterState, AdvertisingOptions, AdvertisingReport, BleResult, Capabilities,
    CharacteristicHandle, ConnectOptions, ConnectionId, DeviceId, DiscoveredDevice, LinkLimits,
    OperationId, OwnerId, PeerId, PermissionRequest, PermissionState, ResourceSnapshot, ScanId,
    ScanOptions, ServerDefinition, ServerId, ServiceInstance, SubscriptionId, WriteType,
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "payload", rename_all = "camelCase")]
pub enum Command {
    GetState,
    GetCapabilities,
    CheckPermissions,
    RequestPermissions(PermissionRequest),
    StartScan(ScanOptions),
    StopScan {
        scan_id: ScanId,
    },
    Connect {
        device_id: DeviceId,
        options: ConnectOptions,
    },
    Disconnect {
        connection_id: ConnectionId,
    },
    DiscoverServices {
        connection_id: ConnectionId,
    },
    Read {
        connection_id: ConnectionId,
        characteristic: CharacteristicHandle,
    },
    Write {
        connection_id: ConnectionId,
        characteristic: CharacteristicHandle,
        value_base64: String,
        write_type: WriteType,
    },
    Subscribe {
        connection_id: ConnectionId,
        characteristic: CharacteristicHandle,
    },
    Unsubscribe {
        subscription_id: SubscriptionId,
    },
    CreateServer(ServerDefinition),
    CloseServer {
        server_id: ServerId,
    },
    StartAdvertising {
        server_id: ServerId,
        options: AdvertisingOptions,
    },
    StopAdvertising {
        server_id: ServerId,
    },
    SetValue {
        server_id: ServerId,
        characteristic_key: String,
        value_base64: String,
    },
    Notify {
        server_id: ServerId,
        peer_id: PeerId,
        characteristic_key: String,
        value_base64: String,
    },
    Cancel {
        operation_id: OperationId,
    },
    CloseOwner,
    DebugResources,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "payload", rename_all = "camelCase")]
// Keeping replies inline preserves the serialized command contract and avoids
// heap allocation on every capability probe.
#[allow(clippy::large_enum_variant)]
pub enum Reply {
    Empty,
    State(AdapterState),
    Capabilities(Capabilities),
    Permissions(PermissionState),
    ScanStarted {
        scan_id: ScanId,
    },
    Connected {
        connection_id: ConnectionId,
        limits: LinkLimits,
    },
    Services(Vec<ServiceInstance>),
    Bytes {
        value_base64: String,
    },
    SubscriptionStarted {
        subscription_id: SubscriptionId,
    },
    ServerCreated {
        server_id: ServerId,
    },
    AdvertisingStarted(AdvertisingReport),
    Resources(ResourceSnapshot),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "payload", rename_all = "camelCase")]
pub enum Event {
    AdapterStateChanged {
        state: AdapterState,
    },
    ScanResult {
        device: DiscoveredDevice,
    },
    ScanStopped {
        scan_id: ScanId,
    },
    ConnectionClosed {
        connection_id: ConnectionId,
    },
    CharacteristicValue {
        subscription_id: SubscriptionId,
        value_base64: String,
    },
    ServerWrite {
        server_id: ServerId,
        peer_id: Option<PeerId>,
        characteristic_key: String,
        value_base64: String,
    },
    SubscriptionChanged {
        server_id: ServerId,
        peer_id: PeerId,
        characteristic_key: String,
        subscribed: bool,
    },
    CriticalStateLoss {
        resource_id: String,
        reason: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationContext {
    pub operation_id: OperationId,
    pub owner_id: OwnerId,
    pub deadline_millis: Option<u64>,
}

#[async_trait]
pub trait Backend: Send + Sync + 'static {
    async fn execute(&self, context: OperationContext, command: Command) -> BleResult<Reply>;
}
