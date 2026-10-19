//! Zairenkai-Core canonical domain types and the local IPC protocol.
//!
//! Every other Core crate and every client (SDK, App, modules) uses these types; there is
//! one representation of each concept. This crate performs no I/O.

pub mod audit;
pub mod capability;
pub mod decision;
pub mod device;
pub mod module;
pub mod observation;
pub mod principal;
pub mod protocol;
pub mod request;
pub mod resource;
pub mod units;
