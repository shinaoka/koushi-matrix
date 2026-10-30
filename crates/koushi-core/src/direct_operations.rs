//! In-process operations an out-of-band caller (the MCP bridge) performs on a
//! live account runtime (#1524).
//!
//! These types are the public shape of those operations. They live outside the
//! runtime module so account, room and timeline sources can name them without
//! reaching into runtime internals.

/// One attachment written into an account's managed media cache (#1524).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FetchedAttachment {
    /// Absolute path inside Koushi's per-account media cache.
    pub path: std::path::PathBuf,
    pub mime_type: Option<String>,
    pub size_bytes: u64,
}

/// Failure classes for a direct (non-GUI) attachment fetch (#1524).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttachmentFetchFailure {
    /// The account has no ready session.
    AccountInactive,
    /// The room is unknown to this account or not joined.
    RoomNotFound,
    /// The event is unknown to the room.
    EventNotFound,
    /// The event carries no downloadable media.
    Unsupported,
    /// The download itself failed (network, key, or write).
    DownloadFailed,
}

/// Failure classes for a direct (non-GUI) room timeline read (#1530).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RoomMessagesFailure {
    /// The account has no ready session.
    AccountInactive,
    /// The room is unknown to this account or not joined.
    RoomNotFound,
    /// The read did not complete (network, server, or timeout).
    Unavailable,
}

/// One reaction group on a directly read message (#1628).
#[derive(Clone, Eq, PartialEq)]
pub struct DirectMessageReaction {
    pub key: String,
    pub count: u32,
    /// Whether this account is one of the senders.
    pub mine: bool,
}

/// The edit that currently applies to a directly read message (#1628).
#[derive(Clone, Eq, PartialEq)]
pub struct DirectMessageEdit {
    pub event_id: String,
    pub timestamp_ms: u64,
}

/// One message-like event as an out-of-band caller reads it (#1628).
///
/// The body is what the timeline would display, so it already has the latest
/// valid edit applied and any reply fallback removed. A body this device cannot
/// decrypt is `None` with `content_available = false`, which is a successful
/// read, not a failure.
#[derive(Clone, Eq, PartialEq)]
pub struct DirectMessageView {
    pub room_id: String,
    pub event_id: String,
    pub sender_id: String,
    pub timestamp_ms: u64,
    pub kind: koushi_protocol::event::RoomMessageKind,
    pub is_encrypted: bool,
    pub is_redacted: bool,
    pub content_available: bool,
    pub body: Option<String>,
    pub edited: Option<DirectMessageEdit>,
    pub reactions: Vec<DirectMessageReaction>,
    /// More relations exist than the one bounded page this read fetched.
    pub relations_truncated: bool,
    pub thread_root_event_id: Option<String>,
    pub reply_to_event_id: Option<String>,
}

/// The outcome of adding a reaction with set semantics (#1628).
#[derive(Clone, Eq, PartialEq)]
pub struct DirectReactionOutcome {
    /// False when this account already had a reaction with that key.
    pub added: bool,
    pub reaction_event_id: Option<String>,
}

/// The outcome of replacing an own message's display text (#1628).
#[derive(Clone, Eq, PartialEq)]
pub struct DirectEditOutcome {
    pub edit_event_id: Option<String>,
}

/// The outcome of redacting an own message (#1628).
#[derive(Clone, Eq, PartialEq)]
pub struct DirectRedactOutcome {
    pub redaction_event_id: Option<String>,
    /// The target was already redacted, so no new event was sent.
    pub already_redacted: bool,
}

/// The outcome of posting a thread reply for an out-of-band caller (#1628).
#[derive(Clone, Eq, PartialEq)]
pub struct DirectThreadReplyOutcome {
    pub event_id: String,
    pub transaction_id: String,
}

/// `Debug` for these shapes records presence only: a body, a reaction key, a
/// sender, and room/event/transaction ids are user content or private account
/// data (REPOSITORY_RULES "Security Rules").
impl std::fmt::Debug for DirectMessageReaction {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DirectMessageReaction")
            .field("key", &"(redacted)")
            .field("count", &self.count)
            .field("mine", &self.mine)
            .finish()
    }
}

impl std::fmt::Debug for DirectMessageEdit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DirectMessageEdit")
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for DirectMessageView {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DirectMessageView")
            .field("kind", &self.kind)
            .field("is_encrypted", &self.is_encrypted)
            .field("is_redacted", &self.is_redacted)
            .field("content_available", &self.content_available)
            .field("reactions", &self.reactions.len())
            .field("edited", &self.edited.is_some())
            .field("relations_truncated", &self.relations_truncated)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for DirectReactionOutcome {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DirectReactionOutcome")
            .field("added", &self.added)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for DirectEditOutcome {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DirectEditOutcome")
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for DirectRedactOutcome {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DirectRedactOutcome")
            .field("already_redacted", &self.already_redacted)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for DirectThreadReplyOutcome {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DirectThreadReplyOutcome")
            .finish_non_exhaustive()
    }
}

/// An out-of-band caller's retry key for an action that sends an event
/// (#1628, #1681). The account runtime answers a repeat of the same key with
/// the first result instead of acting again.
///
/// `Debug` records presence only: the client name and the request id are
/// caller-chosen labels.
#[derive(Clone, Eq, PartialEq)]
pub struct DirectRetryKey {
    /// The calling assistant's validated client name.
    pub client: String,
    pub client_request_id: String,
}

impl std::fmt::Debug for DirectRetryKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DirectRetryKey")
            .finish_non_exhaustive()
    }
}

/// What a direct (non-GUI) message operation asks the account to do (#1628).
///
/// `Debug` records the case only: a body and a reaction key are user content.
#[derive(Clone)]
pub enum DirectMessageRequest {
    Read {
        room_id: String,
        event_id: String,
    },
    React {
        room_id: String,
        event_id: String,
        key: String,
    },
    Unreact {
        room_id: String,
        event_id: String,
        key: String,
    },
    Edit {
        room_id: String,
        event_id: String,
        body: String,
        formatting: koushi_state::ComposerFormattingOptions,
        retry: Option<DirectRetryKey>,
    },
    Delete {
        room_id: String,
        event_id: String,
    },
    ThreadReply {
        room_id: String,
        root_event_id: String,
        body: String,
        formatting: koushi_state::ComposerFormattingOptions,
        retry: Option<DirectRetryKey>,
    },
}

impl std::fmt::Debug for DirectMessageRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Read { .. } => "DirectMessageRequest::Read",
            Self::React { .. } => "DirectMessageRequest::React",
            Self::Unreact { .. } => "DirectMessageRequest::Unreact",
            Self::Edit { .. } => "DirectMessageRequest::Edit",
            Self::Delete { .. } => "DirectMessageRequest::Delete",
            Self::ThreadReply { .. } => "DirectMessageRequest::ThreadReply",
        })
    }
}

/// The settled answer to one [`DirectMessageRequest`].
///
/// `Debug` records the case only: a read carries a message body.
#[derive(Clone)]
pub enum DirectMessageResponse {
    Read(DirectMessageView),
    React(DirectReactionOutcome),
    Unreact(u32),
    Edit(DirectEditOutcome),
    Delete(DirectRedactOutcome),
    ThreadReply(DirectThreadReplyOutcome),
}

impl std::fmt::Debug for DirectMessageResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("DirectMessageResponse(redacted)")
    }
}

/// Failure classes for a direct message read or action (#1628).
///
/// The GUI's own failure kinds are not reused: an out-of-band caller needs to
/// tell a permission mistake from a missing target or a bounded-read limit
/// without reading prose.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DirectMessageFailure {
    /// The account has no ready session.
    AccountInactive,
    /// The room is unknown to this account or not joined.
    RoomNotFound,
    /// The event is unknown to the room.
    EventNotFound,
    /// The event was sent by another user; only own messages can be changed.
    NotAuthor,
    /// The event is a state event or another non-message event.
    NotAMessage,
    /// The target has already been redacted.
    Redacted,
    /// The target is itself a thread reply, not a thread root.
    NotThreadRoot,
    /// One bounded relations page could not establish the reaction set this
    /// action has to decide on, so nothing was sent.
    RelationsUnavailable,
    /// This account already runs the maximum number of direct actions, or its
    /// retry ledger is full of requests whose retry may not act yet.
    Capacity,
    /// A request with the same retry key is still running or ended without a
    /// known outcome, so this one did nothing (#1660).
    DuplicatePending,
    /// The operation did not complete (network, server, or timeout).
    Unavailable,
}

/// An assistant draft to stage with MCP `draft_message` (#1644). The text is
/// written into the target composer's Rust draft store and is never sent.
#[derive(Clone, Eq, PartialEq)]
pub struct AssistantDraftRequest {
    pub target: koushi_state::ComposerTarget,
    pub document: koushi_state::ComposerDocument,
    /// The calling assistant's validated client name, shown by the marker.
    pub client_name: String,
}

impl std::fmt::Debug for AssistantDraftRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AssistantDraftRequest")
            .field("target", &self.target)
            .field("document", &"MessageBody(..)")
            .finish_non_exhaustive()
    }
}

/// A staged assistant draft (#1644).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StagedAssistantDraft {
    /// The revision the draft was written at.
    pub revision: koushi_state::ComposerDraftRevision,
}

/// Why an assistant draft was not staged (#1644). Nothing was written.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssistantDraftFailure {
    /// The account has no ready session.
    AccountInactive,
    /// The room is not in this account's room list.
    RoomNotFound,
    /// The target already has a draft, the renderer is writing or sending it,
    /// or the open composer has a pending submission.
    Conflict,
    /// The account's draft store has not loaded (or failed to load).
    Unavailable,
}
