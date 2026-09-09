//! Platform-neutral contracts for `tauri-plugin-ble`.
//!
//! The crate deliberately contains no Tauri, OS SDK, or WebView types.

mod backend;
mod error;
mod manager;
mod mock;
mod model;

pub use backend::{Backend, Command, Event, OperationContext, Reply};
pub use error::{BleError, BleResult, DeliveryOutcome, ErrorCode};
pub use manager::Manager;
pub use mock::{FakeClock, MockBackend};
pub use model::*;

