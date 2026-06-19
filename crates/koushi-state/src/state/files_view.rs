use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AttachmentKind {
    Image,
    Video,
    Audio,
    File,
    Sticker,
}

impl AttachmentKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::File => "file",
            Self::Sticker => "sticker",
        }
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AttachmentScope {
    Room {
        room_id: String,
    },
    Space {
        space_id: String,
        child_room_ids: Vec<String>,
    },
    Account,
}

impl fmt::Debug for AttachmentScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Room { .. } => formatter
                .debug_struct("AttachmentScope::Room")
                .field("room_id", &"RoomId(..)")
                .finish(),
            Self::Space { child_room_ids, .. } => formatter
                .debug_struct("AttachmentScope::Space")
                .field("space_id", &"RoomId(..)")
                .field("child_room_count", &child_room_ids.len())
                .finish(),
            Self::Account => formatter.write_str("AttachmentScope::Account"),
        }
    }
}

#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct AttachmentFilter {
    pub kinds: Vec<AttachmentKind>,
    pub filename_query: Option<String>,
}

impl fmt::Debug for AttachmentFilter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AttachmentFilter")
            .field("kinds", &self.kinds)
            .field(
                "filename_query",
                &self.filename_query.as_ref().map(|_| "QueryText(..)"),
            )
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AttachmentSort {
    #[default]
    NewestFirst,
    OldestFirst,
    Sender,
    Filename,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct AttachmentResult {
    pub event_id: String,
    pub filename: String,
    pub kind: AttachmentKind,
    pub mimetype: Option<String>,
    pub room_id: String,
    pub sender: String,
    pub size: Option<u64>,
    pub source_mxc: String,
    pub thumbnail_mxc: Option<String>,
    pub timestamp_ms: u64,
    pub thread_root: Option<String>,
    pub encrypted: bool,
    pub encryption_version: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub is_edited: bool,
}

impl fmt::Debug for AttachmentResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AttachmentResult")
            .field("event_id", &"EventId(..)")
            .field("filename", &"AttachmentFilename(..)")
            .field("kind", &self.kind)
            .field("mimetype", &self.mimetype)
            .field("room_id", &"RoomId(..)")
            .field("sender", &"UserId(..)")
            .field("size", &self.size)
            .field("source_mxc", &"MxcUri(..)")
            .field(
                "thumbnail_mxc",
                &self.thumbnail_mxc.as_ref().map(|_| "MxcUri(..)"),
            )
            .field("timestamp_ms", &self.timestamp_ms)
            .field(
                "thread_root",
                &self.thread_root.as_ref().map(|_| "EventId(..)"),
            )
            .field("encrypted", &self.encrypted)
            .field("encryption_version", &self.encryption_version)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("is_edited", &self.is_edited)
            .finish()
    }
}

/// User-facing scope used when opening the files view. The reducer resolves a
/// space scope into the concrete child room ids before querying the index.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FilesViewScope {
    Room { room_id: String },
    Space { space_id: String },
    Account,
}

#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FilesViewState {
    #[default]
    Closed,
    Loading {
        request_id: u64,
        scope: AttachmentScope,
        filter: AttachmentFilter,
        sort: AttachmentSort,
    },
    Open {
        request_id: u64,
        scope: AttachmentScope,
        filter: AttachmentFilter,
        sort: AttachmentSort,
        items: Vec<AttachmentResult>,
        selected_event_id: Option<String>,
    },
    Failed {
        request_id: u64,
        scope: AttachmentScope,
        filter: AttachmentFilter,
        sort: AttachmentSort,
        message: String,
    },
}

impl fmt::Debug for FilesViewState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => formatter.write_str("FilesViewState::Closed"),
            Self::Loading {
                request_id,
                scope,
                filter,
                sort,
            } => formatter
                .debug_struct("FilesViewState::Loading")
                .field("request_id", request_id)
                .field("scope", scope)
                .field("filter", filter)
                .field("sort", sort)
                .finish(),
            Self::Open {
                request_id,
                scope,
                filter,
                sort,
                items,
                selected_event_id,
            } => formatter
                .debug_struct("FilesViewState::Open")
                .field("request_id", request_id)
                .field("scope", scope)
                .field("filter", filter)
                .field("sort", sort)
                .field("items", &format_args!("{} item(s)", items.len()))
                .field(
                    "selected_event_id",
                    &selected_event_id.as_ref().map(|_| "EventId(..)"),
                )
                .finish(),
            Self::Failed {
                request_id,
                scope,
                filter,
                sort,
                message,
            } => formatter
                .debug_struct("FilesViewState::Failed")
                .field("request_id", request_id)
                .field("scope", scope)
                .field("filter", filter)
                .field("sort", sort)
                .field("message", message)
                .finish(),
        }
    }
}
