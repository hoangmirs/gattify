//! Versioned complete-message transport over constrained GATT values.
//!
//! This crate performs no radio I/O. Callers submit the returned frames using a
//! targeted GATT write or notification and feed received values into Receiver.

mod frame;
mod receiver;
mod sender;

pub use frame::{fragment, Frame, FrameKind, HEADER_LEN, PROTOCOL_MAJOR};
pub use receiver::{ReceiveAction, Receiver, ReceiverLimits};
pub use sender::{QueuedMessage, SendAction, SendLimits, Sender};

/// Independently generated UUIDs for the v1 profile characteristics.
pub const INFO_CHARACTERISTIC_UUID: &str = "b1e10f10-6a2c-4a62-8e9e-2c938fa30101";
pub const RX_CHARACTERISTIC_UUID: &str = "b1e10f10-6a2c-4a62-8e9e-2c938fa30102";
pub const TX_CHARACTERISTIC_UUID: &str = "b1e10f10-6a2c-4a62-8e9e-2c938fa30103";

