#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::let_underscore_must_use
)]
//! QuarterMaster Tauri shell: thin typed commands over `qm-service` plus event forwarding.

pub mod bindings;
pub mod commands;
pub mod logging;
pub mod sink;
