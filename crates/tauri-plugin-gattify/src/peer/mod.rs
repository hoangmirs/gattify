//! Versioned complete-message transport over constrained GATT values.
//!
//! The frame, sender and receiver types perform no radio I/O. [`PeerDriver`]
//! runs them over the GATT commands and events of a backend.

mod driver;
#[cfg(test)]
mod driver_tests;
mod frame;
mod receiver;
mod sender;

pub use driver::{
    peer_label, peer_owner, CloseReason, EndpointOptions, PeerDriver, PeerEmitter, PeerEvent,
    SendReceipt, MAX_LOGICAL_PAYLOAD,
};
pub use frame::{fragment, Frame, FrameKind, HEADER_LEN, PROTOCOL_MAJOR};
pub use receiver::{ReceiveAction, Receiver, ReceiverLimits};
pub use sender::{QueuedMessage, SendAction, SendLimits, Sender};

/// UUIDs for the v1 profile characteristics.
pub const INFO_CHARACTERISTIC_UUID: &str = "b1e10f10-6a2c-4a62-8e9e-2c938fa30101";
pub const RX_CHARACTERISTIC_UUID: &str = "b1e10f10-6a2c-4a62-8e9e-2c938fa30102";
pub const TX_CHARACTERISTIC_UUID: &str = "b1e10f10-6a2c-4a62-8e9e-2c938fa30103";
