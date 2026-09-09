use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    AdapterState, AdvertisingOptions, AdvertisingReport, BleResult, Capabilities,
    CharacteristicHandle, ConnectOptions, ConnectionId, DeviceId, DiscoveredDevice, LinkLimits,
    OperationId, OwnerId, PeerId, PermissionRequest, PermissionState, ResourceSnapshot, ScanId,
    ScanOptions, ServerDefinition, ServerId, ServiceInstance, SubscriptionId, WriteType,
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    content = "payload",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
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
#[serde(
    tag = "kind",
    content = "payload",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
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
#[serde(
    tag = "kind",
    content = "payload",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_fields_match_the_typescript_wire_contract() {
        let command: Command =
            serde_json::from_str(r#"{"kind":"stopScan","payload":{"scanId":"scan-1"}}"#).unwrap();
        assert_eq!(
            command,
            Command::StopScan {
                scan_id: ScanId::new("scan-1")
            }
        );
    }

    #[test]
    fn reply_and_event_fields_use_camel_case() {
        let reply = serde_json::to_value(Reply::ScanStarted {
            scan_id: ScanId::new("scan-1"),
        })
        .unwrap();
        assert_eq!(reply["payload"]["scanId"], "scan-1");

        let event = serde_json::to_value(Event::CharacteristicValue {
            subscription_id: SubscriptionId::new("subscription-1"),
            value_base64: "AA==".into(),
        })
        .unwrap();
        assert_eq!(event["payload"]["subscriptionId"], "subscription-1");
        assert_eq!(event["payload"]["valueBase64"], "AA==");
    }
}
