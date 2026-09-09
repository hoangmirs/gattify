use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{OperationId, ResourceId};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    PermissionDenied,
    BluetoothOff,
    Unavailable,
    Unsupported,
    InvalidArgument,
    InvalidHandle,
    Busy,
    Timeout,
    Disconnected,
    PayloadTooLarge,
    QueueFull,
    ProtocolMismatch,
    Cancelled,
    Internal,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DeliveryOutcome {
    NotSubmitted,
    Unknown,
    TransportAcknowledged,
}

#[derive(Clone, Debug, Deserialize, Error, Eq, PartialEq, Serialize)]
#[error("{code:?}: {message}")]
#[serde(rename_all = "camelCase")]
pub struct BleError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<OperationId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<ResourceId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery: Option<DeliveryOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_code: Option<String>,
}

impl BleError {
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            operation_id: None,
            resource_id: None,
            delivery: None,
            native_code: None,
        }
    }

    #[must_use]
    pub fn unsupported(reason: impl Into<String>) -> Self {
        Self::new(ErrorCode::Unsupported, reason)
    }

    #[must_use]
    pub fn invalid_handle(resource: impl Into<ResourceId>) -> Self {
        let resource_id = resource.into();
        Self {
            code: ErrorCode::InvalidHandle,
            message: "resource handle is stale, unknown, or belongs to another owner".into(),
            operation_id: None,
            resource_id: Some(resource_id),
            delivery: None,
            native_code: None,
        }
    }

    #[must_use]
    pub fn with_operation(mut self, operation_id: OperationId) -> Self {
        self.operation_id = Some(operation_id);
        self
    }
}

pub type BleResult<T> = Result<T, BleError>;
