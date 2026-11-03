use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomInteractionState {
    pub pinned_events: Vec<PinnedEvent>,
    pub pin_operation: PinOperationState,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PinnedEvent {
    pub event_id: String,
    pub sender: Option<String>,
    #[serde(default)]
    pub sender_label: Option<String>,
    pub body_preview: Option<String>,
    pub redacted: bool,
    #[serde(default)]
    pub timestamp_ms: Option<u64>,
    #[serde(default)]
    pub state: PinnedEventState,
    #[serde(default)]
    pub thread_root_event_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PinnedEventState {
    #[default]
    Ready,
    UnableToDecrypt,
    Unavailable,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReplyQuoteCodeBlock {
    pub language: Option<String>,
    pub body: String,
}

impl fmt::Debug for ReplyQuoteCodeBlock {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReplyQuoteCodeBlock")
            .field(
                "language",
                &self.language.as_ref().map(|_| "CodeBlockLanguage(..)"),
            )
            .field("body", &"CodeBlockBody(..)")
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReplyQuoteFormattedBody {
    pub html: String,
    pub plain_text: String,
    pub code_blocks: Vec<ReplyQuoteCodeBlock>,
}

impl fmt::Debug for ReplyQuoteFormattedBody {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReplyQuoteFormattedBody")
            .field("html", &"FormattedHtml(..)")
            .field("plain_text", &"FormattedPlainText(..)")
            .field("code_blocks", &self.code_blocks.len())
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReplyQuote {
    pub event_id: String,
    pub sender: Option<String>,
    #[serde(default)]
    pub sender_label: Option<String>,
    pub body_preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formatted: Option<ReplyQuoteFormattedBody>,
    /// #1328: the quoted message's `m.mentions` user ids, so the quote preview
    /// marks its anchors as the pills its own row would.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mentioned_user_ids: Vec<String>,
    /// #1769: public media metadata for a quoted media original. React renders
    /// the thumbnail slot from this; private sources stay in the actor ledger.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<ReplyQuoteMedia>,
    /// The canonical `matrix.to` link Rust projects for the quoted event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permalink: Option<String>,
    /// Whether the quote's current state may open the original. Only `Ready`
    /// and `Redacted` quotes are openable; unresolved and terminal states stay
    /// inert.
    #[serde(default)]
    pub can_open: bool,
    pub state: ReplyQuoteState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReplyQuoteMediaKind {
    Image,
    Sticker,
    Video,
    File,
    Audio,
}

/// Public metadata only: no MXC URI, no encryption keys, no cache ref.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReplyQuoteMedia {
    pub kind: ReplyQuoteMediaKind,
    #[serde(default)]
    pub animated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u64>,
    #[serde(default)]
    pub has_thumbnail_source: bool,
}

impl fmt::Debug for ReplyQuote {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReplyQuote")
            .field("event_id", &"EventId(..)")
            .field("sender", &self.sender.as_ref().map(|_| "UserId(..)"))
            .field(
                "sender_label",
                &self.sender_label.as_ref().map(|_| "SenderLabel(..)"),
            )
            .field(
                "body_preview",
                &self.body_preview.as_ref().map(|_| "BodyPreview(..)"),
            )
            .field(
                "formatted",
                &self
                    .formatted
                    .as_ref()
                    .map(|_| "ReplyQuoteFormattedBody(..)"),
            )
            .field("mentioned_user_ids", &self.mentioned_user_ids.len())
            .field("media", &self.media.as_ref().map(|media| media.kind))
            .field(
                "permalink",
                &self.permalink.as_ref().map(|_| "Permalink(..)"),
            )
            .field("can_open", &self.can_open)
            .field("state", &self.state)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReplyQuoteState {
    /// The original is unresolved and Rust still owns a bounded lookup.
    Loading,
    Ready,
    Redacted,
    /// Terminal: the homeserver reported the original as not found or forbidden.
    Missing,
    Unsupported,
    /// Terminal for this timeline actor: the bounded lookup exhausted retries.
    Failed,
}

impl ReplyQuoteState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Loading => "loading",
            Self::Ready => "ready",
            Self::Redacted => "redacted",
            Self::Missing => "missing",
            Self::Unsupported => "unsupported",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PinOp {
    Pin,
    Unpin,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PinOperationState {
    #[default]
    Idle,
    Pending {
        request_id: u64,
        room_id: String,
        event_id: String,
        op: PinOp,
    },
    Failed {
        room_id: String,
        event_id: String,
        op: PinOp,
        recoverable: bool,
    },
}

impl PinOperationState {
    pub fn request_id(&self) -> Option<u64> {
        match self {
            Self::Idle => None,
            Self::Pending { request_id, .. } => Some(*request_id),
            Self::Failed { .. } => None,
        }
    }

    pub fn is_idle(&self) -> bool {
        matches!(self, Self::Idle)
    }

    pub fn accepts_new_request(&self) -> bool {
        matches!(
            self,
            Self::Idle
                | Self::Failed {
                    recoverable: true,
                    ..
                }
        )
    }
}
