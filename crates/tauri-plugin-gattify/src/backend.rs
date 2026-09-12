use std::sync::Arc;

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
        max_value_length: Option<u32>,
    },
    CriticalStateLoss {
        resource_id: String,
        reason: String,
    },
}

/// One event from the native layer, with the owner of its resource.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventEnvelope {
    pub owner_id: OwnerId,
    pub event: Event,
}

/// Receives every event a backend raises, with the owner of its resource.
///
/// A backend gets its sink when it is constructed. The sink must not block:
/// native callbacks call it on platform threads.
pub type EventSink = Arc<dyn Fn(OwnerId, Event) + Send + Sync>;

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

#[async_trait]
impl Backend for std::sync::Arc<dyn Backend> {
    async fn execute(&self, context: OperationContext, command: Command) -> BleResult<Reply> {
        self.as_ref().execute(context, command).await
    }
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

    #[test]
    fn native_event_envelopes_parse() {
        let envelope: EventEnvelope = serde_json::from_str(
            r#"{"ownerId":"webview:main","event":{"kind":"scanStopped","payload":{"scanId":"scan-1"}}}"#,
        )
        .unwrap();
        assert_eq!(
            envelope,
            EventEnvelope {
                owner_id: OwnerId::new("webview:main"),
                event: Event::ScanStopped {
                    scan_id: ScanId::new("scan-1")
                },
            }
        );
    }

    #[test]
    fn subscription_changes_carry_the_notification_size() {
        let event: Event = serde_json::from_value(serde_json::json!({
            "kind": "subscriptionChanged",
            "payload": {
                "serverId": "server-1",
                "peerId": "central-1",
                "characteristicKey": "peer/tx",
                "subscribed": true,
                "maxValueLength": 182
            }
        }))
        .unwrap();
        assert_eq!(
            event,
            Event::SubscriptionChanged {
                server_id: ServerId::new("server-1"),
                peer_id: PeerId::new("central-1"),
                characteristic_key: "peer/tx".into(),
                subscribed: true,
                max_value_length: Some(182),
            }
        );
    }
}
