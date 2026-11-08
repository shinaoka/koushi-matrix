//! Timeline media download state types — #78.
//!
//! Moved here from `state.rs` per the #87-alignment principle: new cohesive
//! units live in their own files, re-exported from the parent module for
//! backward compatibility with existing callers.

use serde::{Deserialize, Serialize};

use super::OperationFailureKind;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MediaTransferProgress {
    pub current: u64,
    pub total: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TimelineMediaDownloadState {
    #[default]
    NotRequested,
    Pending {
        progress: Option<MediaTransferProgress>,
    },
    Ready {
        /// Opaque app-owned cache handle: `download/<room-hash>/<event-hash>`
        /// for the original,
        /// `download/<room-hash>/<event-hash>/<width>x<height>` for a
        /// thumbnail. Never a filesystem path; the platform adapter resolves
        /// it.
        source_ref: String,
        width: Option<u64>,
        height: Option<u64>,
        mime_type: Option<String>,
    },
    Failed {
        failure_kind: OperationFailureKind,
    },
}
