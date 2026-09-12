use std::{fmt, str::FromStr, time::Duration};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde::{Deserialize, Serialize};

use crate::{BleError, BleResult, ErrorCode};

macro_rules! opaque_id {
    ($name:ident) => {
        #[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            #[must_use]
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }
    };
}

opaque_id!(AdapterId);
opaque_id!(DeviceId);
opaque_id!(ConnectionId);
opaque_id!(ServerId);
opaque_id!(PeerId);
opaque_id!(ScanId);
opaque_id!(SubscriptionId);
opaque_id!(OperationId);
opaque_id!(OwnerId);
opaque_id!(ServiceHandle);
opaque_id!(CharacteristicHandle);
opaque_id!(ResourceId);

impl From<ScanId> for ResourceId {
    fn from(value: ScanId) -> Self {
        Self(value.0)
    }
}

impl From<ConnectionId> for ResourceId {
    fn from(value: ConnectionId) -> Self {
        Self(value.0)
    }
}

impl From<SubscriptionId> for ResourceId {
    fn from(value: SubscriptionId) -> Self {
        Self(value.0)
    }
}

impl From<ServerId> for ResourceId {
    fn from(value: ServerId) -> Self {
        Self(value.0)
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AdapterState {
    #[default]
    Unknown,
    Unavailable,
    Unauthorized,
    PoweredOff,
    Resetting,
    PoweredOn,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SupportLevel {
    Supported,
    Unsupported,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Support {
    pub level: SupportLevel,
    pub reason: String,
    pub description: Option<String>,
}

impl Support {
    #[must_use]
    pub fn supported() -> Self {
        Self {
            level: SupportLevel::Supported,
            reason: "available".into(),
            description: None,
        }
    }

    #[must_use]
    pub fn unsupported(reason: impl Into<String>) -> Self {
        Self {
            level: SupportLevel::Unsupported,
            reason: reason.into(),
            description: None,
        }
    }

    #[must_use]
    pub fn unknown(reason: impl Into<String>) -> Self {
        Self {
            level: SupportLevel::Unknown,
            reason: reason.into(),
            description: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub central: Support,
    pub peripheral: Support,
    pub advertising: Support,
    pub targeted_notify: Support,
    pub simultaneous_roles: Support,
    pub background: Support,
    pub max_connections: Option<u32>,
    pub max_advertising_data_length: Option<u32>,
}

impl Capabilities {
    #[must_use]
    pub fn unknown() -> Self {
        let unknown = || Support::unknown("notProbed");
        Self {
            central: unknown(),
            peripheral: unknown(),
            advertising: unknown(),
            targeted_notify: unknown(),
            simultaneous_roles: unknown(),
            background: unknown(),
            max_connections: None,
            max_advertising_data_length: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionOutcome {
    Granted,
    Promptable,
    DeniedPermanently,
    Restricted,
    NotRequired,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionState {
    pub scan: PermissionOutcome,
    pub connect: PermissionOutcome,
    pub advertise: PermissionOutcome,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRequest {
    pub scan: bool,
    pub connect: bool,
    pub advertise: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdvertisementData {
    pub local_name: Option<String>,
    pub service_data: Vec<ServiceData>,
    pub manufacturer_data: Vec<ManufacturerData>,
    pub connectable: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceData {
    pub service_uuid: String,
    pub bytes_base64: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManufacturerData {
    pub company_id: u16,
    pub bytes_base64: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredDevice {
    pub id: DeviceId,
    pub name: Option<String>,
    pub rssi: Option<i16>,
    pub service_uuids: Vec<String>,
    pub advertisement: Option<AdvertisementData>,
    pub observed_at_millis: u64,
    pub scan_id: ScanId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkLimits {
    pub write_with_response: Option<u32>,
    pub write_without_response: Option<u32>,
    pub notification: Option<u32>,
    pub att_mtu: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanOptions {
    pub service_uuids: Vec<String>,
    pub timeout_ms: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectOptions {
    pub timeout_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum WriteType {
    WithResponse,
    WithoutResponse,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceInstance {
    pub handle: ServiceHandle,
    pub uuid: String,
    pub characteristics: Vec<CharacteristicInstance>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacteristicInstance {
    pub handle: CharacteristicHandle,
    pub uuid: String,
    pub properties: CharacteristicProperties,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
// These independent flags mirror the BLE characteristic property bitfield.
#[allow(clippy::struct_excessive_bools)]
pub struct CharacteristicProperties {
    pub read: bool,
    pub write: bool,
    pub write_without_response: bool,
    pub notify: bool,
    pub indicate: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerDefinition {
    pub services: Vec<LocalService>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalService {
    pub instance_key: String,
    pub uuid: String,
    pub primary: bool,
    pub characteristics: Vec<LocalCharacteristic>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalCharacteristic {
    pub instance_key: String,
    pub uuid: String,
    pub properties: CharacteristicProperties,
    pub initial_value_base64: Option<String>,
    pub max_value_length: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdvertisingOptions {
    pub service_uuid: String,
    pub local_name: Option<String>,
    pub local_name_optional: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdvertisingReport {
    pub local_name_included: bool,
    pub local_name_truncated: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ScanState {
    Starting,
    Active,
    Stopping,
    Stopped,
    Failed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionState {
    Connecting,
    Connected,
    Disconnecting,
    Closed,
    Failed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ServerState {
    Registering,
    Ready,
    Advertising,
    Closing,
    Closed,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceSnapshot {
    pub scans: usize,
    pub connections: usize,
    pub subscriptions: usize,
    pub servers: usize,
}

#[derive(Clone, Debug)]
pub struct Deadlines {
    pub connect: Duration,
    pub discovery: Duration,
    pub procedure: Duration,
}

impl Default for Deadlines {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(15),
            discovery: Duration::from_secs(10),
            procedure: Duration::from_secs(5),
        }
    }
}

/// Validates a canonical 128-bit hexadecimal UUID, with or without hyphens.
///
/// # Errors
///
/// Returns [`ErrorCode::InvalidArgument`] when `value` is not a 128-bit UUID.
pub fn validate_uuid(value: &str) -> BleResult<()> {
    let compact: String = value
        .chars()
        .filter(|character| *character != '-')
        .collect();
    if compact.len() != 32
        || !compact
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err(BleError::new(
            ErrorCode::InvalidArgument,
            "UUID must be a 128-bit hexadecimal UUID",
        ));
    }
    Ok(())
}

/// Validates a local GATT server definition before it reaches a platform SDK.
///
/// # Errors
///
/// Returns [`ErrorCode::InvalidArgument`] for empty definitions, invalid UUIDs,
/// duplicate instance keys, instance keys with a `/`, zero maximum lengths, or
/// invalid initial values.
pub fn validate_server_definition(definition: &ServerDefinition) -> BleResult<()> {
    use std::collections::HashSet;

    if definition.services.is_empty() {
        return Err(BleError::new(
            ErrorCode::InvalidArgument,
            "at least one service is required",
        ));
    }
    // A characteristic key joins the two instance keys with `/`.
    let check_key = |key: &str| {
        if key.contains('/') {
            Err(BleError::new(
                ErrorCode::InvalidArgument,
                "instance keys must not contain '/'",
            ))
        } else {
            Ok(())
        }
    };
    let mut keys = HashSet::new();
    for service in &definition.services {
        validate_uuid(&service.uuid)?;
        check_key(&service.instance_key)?;
        if !keys.insert(format!("service:{}", service.instance_key)) {
            return Err(BleError::new(
                ErrorCode::InvalidArgument,
                "duplicate service instance key",
            ));
        }
        for characteristic in &service.characteristics {
            validate_uuid(&characteristic.uuid)?;
            check_key(&characteristic.instance_key)?;
            if characteristic.max_value_length == 0 {
                return Err(BleError::new(
                    ErrorCode::InvalidArgument,
                    "characteristic max value length must be positive",
                ));
            }
            if !keys.insert(format!(
                "characteristic:{}:{}",
                service.instance_key, characteristic.instance_key
            )) {
                return Err(BleError::new(
                    ErrorCode::InvalidArgument,
                    "duplicate characteristic instance key within service",
                ));
            }
            if let Some(initial) = &characteristic.initial_value_base64 {
                let decoded = BASE64.decode(initial).map_err(|_| {
                    BleError::new(
                        ErrorCode::InvalidArgument,
                        "characteristic initial value is not valid base64",
                    )
                })?;
                if decoded.len() > characteristic.max_value_length as usize {
                    return Err(BleError::new(
                        ErrorCode::InvalidArgument,
                        "characteristic initial value exceeds its maximum length",
                    ));
                }
            }
        }
    }
    Ok(())
}

impl FromStr for AdapterState {
    type Err = BleError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "unknown" => Ok(Self::Unknown),
            "unavailable" => Ok(Self::Unavailable),
            "unauthorized" => Ok(Self::Unauthorized),
            "poweredOff" => Ok(Self::PoweredOff),
            "resetting" => Ok(Self::Resetting),
            "poweredOn" => Ok(Self::PoweredOn),
            _ => Err(BleError::new(
                ErrorCode::InvalidArgument,
                "unknown adapter state",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn definition(initial: Option<&str>, max_value_length: u32) -> ServerDefinition {
        ServerDefinition {
            services: vec![LocalService {
                instance_key: "s".into(),
                uuid: "b1e10f10-6a2c-4a62-8e9e-2c938fa30100".into(),
                primary: true,
                characteristics: vec![LocalCharacteristic {
                    instance_key: "c".into(),
                    uuid: "b1e10f10-6a2c-4a62-8e9e-2c938fa30101".into(),
                    properties: CharacteristicProperties::default(),
                    initial_value_base64: initial.map(Into::into),
                    max_value_length,
                }],
            }],
        }
    }

    #[test]
    fn instance_keys_cannot_hold_the_key_separator() {
        let mut slashed_service = definition(None, 20);
        slashed_service.services[0].instance_key = "a/b".into();
        assert_eq!(
            validate_server_definition(&slashed_service)
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        let mut slashed_characteristic = definition(None, 20);
        slashed_characteristic.services[0].characteristics[0].instance_key = "b/c".into();
        assert_eq!(
            validate_server_definition(&slashed_characteristic)
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn initial_values_are_validated_before_reaching_a_platform_sdk() {
        assert!(validate_server_definition(&definition(Some("AAEC"), 20)).is_ok());
        assert_eq!(
            validate_server_definition(&definition(Some("not base64!"), 20))
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            validate_server_definition(&definition(Some("AAEC"), 2))
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
    }
}
