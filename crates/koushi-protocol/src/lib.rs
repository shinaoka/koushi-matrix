#![forbid(unsafe_code)]

//! Transport-neutral public command/event identity and state-update DTOs.

pub mod command;
pub mod event;
pub mod failure;
pub mod ids;
pub mod state_update;
mod u64_decimal_string;
pub mod view;

pub use command::*;
pub use event::*;
pub use failure::*;
pub use ids::*;
pub use state_update::{
    AppStateSnapshot, CoreCommandAdmission, RoomLiveSignalMetadata, StateDelta,
    StateDeltaChangedSlices, VersionedAppStateSnapshot,
};
