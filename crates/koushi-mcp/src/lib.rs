//! Shared implementation of the Koushi MCP bridge.
//!
//! The launcher binary and the Koushi app both depend on this crate: the
//! launcher for the stdio loop, endpoint reader, and socket client; the app for
//! the same wire types and tool definitions. One owner for each contract means
//! the two processes cannot drift.

pub mod cli;
pub mod endpoint;
pub mod ipc;
pub mod server;
pub mod tools;
pub mod wire;

pub use wire::{ErrorCode, WireError};

#[cfg(windows)]
pub mod windows_security;
