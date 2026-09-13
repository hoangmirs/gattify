//! The Windows backend: `WinRT` Bluetooth LE through the `windows` crate.
//!
//! The modules without `WinRT` types hold the rules of the contract: IDs,
//! queues, scan results, ATT limits, the advertisement and the error codes.
//! They compile on every host, so their tests run everywhere.

mod advertise;
mod gatt;
mod ids;
mod queue;
mod scan;
mod status;
