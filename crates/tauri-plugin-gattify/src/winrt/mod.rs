//! The Windows backend: `WinRT` Bluetooth LE through the `windows` crate.
//!
//! One engine thread owns all state, as the serial queue of the iOS backend
//! does. Commands and every `WinRT` callback post jobs to it, and `WinRT`
//! async operations run as tasks that post their results back.
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

#[cfg(all(feature = "tauri", target_os = "windows"))]
mod adapter;
#[cfg(all(feature = "tauri", target_os = "windows"))]
mod backend;
#[cfg(all(feature = "tauri", target_os = "windows"))]
mod central;
#[cfg(all(feature = "tauri", target_os = "windows"))]
mod convert;
#[cfg(all(feature = "tauri", target_os = "windows"))]
mod engine;
#[cfg(all(feature = "tauri", target_os = "windows"))]
mod peripheral;
#[cfg(all(feature = "tauri", target_os = "windows"))]
mod scanner;

#[cfg(all(feature = "tauri", target_os = "windows"))]
pub(crate) use backend::WindowsBackend;
