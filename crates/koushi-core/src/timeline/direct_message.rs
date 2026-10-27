//! One direct (display-independent) message read or action for an out-of-band
//! caller (#1628).
//!
//! The GUI edits, reacts to, and redacts messages through a resident
//! `TimelineActor`, and reads them through its own timeline window. An MCP
//! caller must work without either (#1524): a room or thread nobody opened has
//! no actor, and mounting one would create presentation state. This module
//! therefore runs on the account session directly, like
//! [`super::direct_room_messages`].
//!
//! Display policy is not reimplemented here. The effective body comes from the
//! SDK's own `TimelineItemContent::from_event`, which already applies an edit's
//! `m.new_content` and strips the reply fallback; which replacement is valid
//! comes from `check_validity_of_replacement_events`; and how an edit keeps a
//! media caption comes from [`super::item_projection`].

use std::sync::Arc;
use std::time::Duration;

use koushi_protocol::event::RoomMessageKind;
use koushi_sdk::MatrixClientSession;
use matrix_sdk::RoomState;
use matrix_sdk::config::RequestConfig;
use matrix_sdk::deserialized_responses::TimelineEvent;
use matrix_sdk::ruma::events::room::message::{Relation, RoomMessageEventContent};
use matrix_sdk::ruma::events::{
    AnySyncMessageLikeEvent, AnySyncTimelineEvent, SyncMessageLikeEvent,
};
use matrix_sdk::ruma::html::{HtmlSanitizerMode, RemoveReplyFallback};
use matrix_sdk::ruma::{EventId, UInt};

use crate::direct_operations::{
    DirectEditOutcome, DirectMessageEdit, DirectMessageFailure, DirectMessageReaction,
    DirectMessageView, DirectReactionOutcome, DirectRedactOutcome,
};

/// Bound on one event read, including decryption.
const READ_TIMEOUT: Duration = Duration::from_secs(20);

/// Bound on one relations page (256 relations), and on the request itself.
/// Paging further would let a target with a large history stall the caller.
const PAGE_LIMIT: u32 = 256;
const RELATIONS_TIMEOUT: Duration = Duration::from_secs(20);

fn read_request_config() -> RequestConfig {
    RequestConfig::new().timeout(READ_TIMEOUT)
}

/// Resolves a joined room of this session.
fn joined_room(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<matrix_sdk::Room, DirectMessageFailure> {
    let parsed =
        matrix_sdk::ruma::RoomId::parse(room_id).map_err(|_| DirectMessageFailure::RoomNotFound)?;
    let room = session
        .client()
        .get_room(&parsed)
        .ok_or(DirectMessageFailure::RoomNotFound)?;
    if room.state() != RoomState::Joined {
        return Err(DirectMessageFailure::RoomNotFound);
    }
    Ok(room)
}

fn parse_event_id(event_id: &str) -> Result<matrix_sdk::ruma::OwnedEventId, DirectMessageFailure> {
    EventId::parse(event_id).map_err(|_| DirectMessageFailure::EventNotFound)
}

/// Reads the target of a mutation straight from the homeserver.
///
/// A mutation must not act on a stale cached copy: the local event cache can
/// still hold the pre-redaction event after this account deleted it. One bounded
/// fetch is authoritative, and a mutation is rare enough to afford it.
async fn load_event_authoritative(
    room: &matrix_sdk::Room,
    event_id: &matrix_sdk::ruma::OwnedEventId,
) -> Result<TimelineEvent, DirectMessageFailure> {
    crate::executor::timeout(
        READ_TIMEOUT,
        room.event(event_id, Some(read_request_config())),
    )
    .await
    .map_err(|_| DirectMessageFailure::Unavailable)?
    .map_err(|error| {
        use matrix_sdk::ruma::api::error::ErrorKind;
        match error.client_api_error_kind() {
            Some(ErrorKind::NotFound | ErrorKind::Forbidden) => DirectMessageFailure::EventNotFound,
            _ => DirectMessageFailure::Unavailable,
        }
    })
}

/// The one bounded relations page this read is allowed to fetch.
///
/// Paging to exhaustion would let a target with a large history stall the
/// caller, so a full page reports truncation instead.
async fn relation_page(
    room: &matrix_sdk::Room,
    event_id: &matrix_sdk::ruma::OwnedEventId,
) -> Result<(Vec<TimelineEvent>, bool), DirectMessageFailure> {
    let options = matrix_sdk::room::RelationsOptions {
        limit: Some(UInt::from(PAGE_LIMIT)),
        ..Default::default()
    };
    let relations =
        crate::executor::timeout(RELATIONS_TIMEOUT, room.relations(event_id.clone(), options))
            .await
            .map_err(|_| DirectMessageFailure::RelationsUnavailable)?
            .map_err(|_| DirectMessageFailure::RelationsUnavailable)?;
    // A server may hand back a continuation token even when it returned
    // everything; a page shorter than the limit means there is nothing more to
    // page for, so only a full page is uncertain.
    let truncated =
        relations.next_batch_token.is_some() && relations.chunk.len() >= PAGE_LIMIT as usize;
    Ok((relations.chunk, truncated))
}

/// The message a room event carries, or `None` for anything else.
fn message_of(event: &TimelineEvent) -> Option<SyncMessageLikeEvent<RoomMessageEventContent>> {
    let parsed: AnySyncTimelineEvent = event.raw().deserialize().ok()?;
    match parsed {
        AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::RoomMessage(message)) => {
            Some(message)
        }
        _ => None,
    }
}

/// Whether this event is an encrypted message this device cannot read.
///
/// It is still a message-like event the caller may ask about: the read succeeds
/// with no body instead of claiming the event is not a message.
fn is_undecryptable_message(event: &TimelineEvent) -> bool {
    let Ok(parsed): Result<AnySyncTimelineEvent, _> = event.raw().deserialize() else {
        return false;
    };
    matches!(
        parsed,
        AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::RoomEncrypted(_))
    ) && event.encryption_info().is_none()
}

/// The relation the message carries: its thread root or the event it replies to.
fn relations_of(
    message: &SyncMessageLikeEvent<RoomMessageEventContent>,
) -> (Option<String>, Option<String>) {
    let relates_to = match message {
        SyncMessageLikeEvent::Original(original) => original.content.relates_to.as_ref(),
        SyncMessageLikeEvent::Redacted(_) => None,
    };
    match relates_to {
        Some(Relation::Thread(thread)) => (
            Some(thread.event_id.to_string()),
            thread
                .in_reply_to
                .as_ref()
                .map(|reply| reply.event_id.to_string()),
        ),
        Some(Relation::Reply(reply)) => (None, Some(reply.in_reply_to.event_id.to_string())),
        _ => (None, None),
    }
}

/// The edit that applies to this event, from the bounded relations page.
///
/// Only a replacement the SDK's own validity rules accept can change the
/// displayed text; an invalid one is ignored, exactly as the timeline ignores
/// it.
fn latest_valid_edit<'a>(
    target: &TimelineEvent,
    relations: &'a [TimelineEvent],
) -> Option<&'a TimelineEvent> {
    // Same ordering as the SDK's own replacement picker: newer timestamp wins,
    // and the event id breaks a tie so two edits in the same millisecond are
    // still decided deterministically.
    let mut latest: Option<((u64, String), &'a TimelineEvent)> = None;
    for relation in relations {
        if matrix_sdk::check_validity_of_replacement_events(
            target.raw(),
            target.encryption_info().map(|info| info.as_ref()),
            relation.raw(),
            relation.encryption_info().map(|info| info.as_ref()),
        )
        .is_err()
        {
            continue;
        }
        let Some(timestamp) = relation.timestamp().map(|timestamp| u64::from(timestamp.0)) else {
            continue;
        };
        let event_id = relation
            .event_id()
            .map(ToString::to_string)
            .unwrap_or_default();
        let key = (timestamp, event_id);
        if latest.as_ref().is_none_or(|(current, _)| key > *current) {
            latest = Some((key, relation));
        }
    }
    latest.map(|(_, relation)| relation)
}

/// What one relation event is, for the bounded scans below.
enum ReactionEntry {
    /// A live `m.annotation` by `sender` with `key`.
    Reaction { sender: String, key: String },
    /// A relation this device cannot read (an undecryptable event), so the set
    /// it belongs to cannot be established.
    Undecryptable,
    /// Some other relation (an edit, a thread reply, a redacted reaction).
    Other,
}

fn relation_entry(relation: &TimelineEvent) -> ReactionEntry {
    let Ok(parsed): Result<AnySyncTimelineEvent, _> = relation.raw().deserialize() else {
        return ReactionEntry::Other;
    };
    match parsed {
        AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::Reaction(reaction)) => {
            match reaction {
                SyncMessageLikeEvent::Original(original) => ReactionEntry::Reaction {
                    sender: original.sender.to_string(),
                    key: original.content.relates_to.key.clone(),
                },
                SyncMessageLikeEvent::Redacted(_) => ReactionEntry::Other,
            }
        }
        AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::RoomEncrypted(_)) => {
            ReactionEntry::Undecryptable
        }
        _ => ReactionEntry::Other,
    }
}

/// The reaction groups of one event, from the same bounded relations page.
///
/// One sender counts once per key, the way the timeline's own projection
/// aggregates them.
fn reaction_groups(
    relations: &[TimelineEvent],
    own_user_id: Option<&matrix_sdk::ruma::UserId>,
) -> Vec<DirectMessageReaction> {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for relation in relations {
        let ReactionEntry::Reaction { sender, key } = relation_entry(relation) else {
            continue;
        };
        match groups.iter_mut().find(|(group_key, _)| group_key == &key) {
            Some((_, senders)) => {
                if !senders.contains(&sender) {
                    senders.push(sender);
                }
            }
            None => groups.push((key, vec![sender])),
        }
    }
    groups
        .into_iter()
        .map(|(key, senders)| DirectMessageReaction {
            mine: own_user_id
                .is_some_and(|own| senders.iter().any(|sender| sender == own.as_str())),
            count: senders.len() as u32,
            key,
        })
        .collect()
}

/// Every own reaction event id with `key`, for removal, and whether the page
/// held a relation this device cannot read.
fn own_reaction_event_ids(
    relations: &[TimelineEvent],
    own_user_id: &matrix_sdk::ruma::UserId,
    key: &str,
) -> (Vec<matrix_sdk::ruma::OwnedEventId>, bool) {
    let mut ids = Vec::new();
    let mut unresolved = false;
    for relation in relations {
        if matches!(relation_entry(relation), ReactionEntry::Undecryptable) {
            unresolved = true;
        }
        let Ok(AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::Reaction(reaction))) =
            relation.raw().deserialize()
        else {
            continue;
        };
        let SyncMessageLikeEvent::Original(original) = reaction else {
            continue;
        };
        if original.sender == own_user_id && original.content.relates_to.key == key {
            ids.push(original.event_id.clone());
        }
    }
    (ids, unresolved)
}

/// Whether the event was redacted, and which message kind it is.
fn kind_and_redaction(
    message: &SyncMessageLikeEvent<RoomMessageEventContent>,
) -> (RoomMessageKind, bool) {
    match message {
        SyncMessageLikeEvent::Original(original) => (
            super::direct_room_messages::message_kind(&original.content.msgtype),
            false,
        ),
        SyncMessageLikeEvent::Redacted(_) => (RoomMessageKind::Other, true),
    }
}

/// Reads one message-like event of a joined room for an out-of-band caller.
///
/// The body is the text the timeline would display: the latest valid edit is
/// applied and the reply fallback is removed by the SDK's own projection. This
/// never marks the event read, sends a receipt, or starts a resolver.
pub(crate) async fn read_direct_message(
    session: Arc<MatrixClientSession>,
    room_id: &str,
    event_id: &str,
) -> Result<DirectMessageView, DirectMessageFailure> {
    let room = joined_room(&session, room_id)?;
    let event_id = EventId::parse(event_id).map_err(|_| DirectMessageFailure::EventNotFound)?;
    // One bounded authoritative read: the local event cache can still hold a
    // pre-redaction copy of an event this account just deleted, and a caller
    // asking for one event can afford the request.
    let target = load_event_authoritative(&room, &event_id).await?;
    let undecryptable = is_undecryptable_message(&target);
    let message = match message_of(&target) {
        Some(message) => Some(message),
        // A message this device cannot decrypt is a successful read with no
        // body, not "not a message".
        None if undecryptable => None,
        None => return Err(DirectMessageFailure::NotAMessage),
    };
    let (kind, is_redacted) = message
        .as_ref()
        .map(kind_and_redaction)
        .unwrap_or((RoomMessageKind::Other, false));
    let (thread_root_event_id, reply_to_event_id) =
        message.as_ref().map(relations_of).unwrap_or((None, None));
    let timestamp_ms = target
        .timestamp()
        .map(|timestamp| u64::from(timestamp.0))
        .unwrap_or_default();
    // The envelope names the sender even when this device cannot read the
    // content.
    let sender_id = target
        .raw()
        .deserialize()
        .map(|event: AnySyncTimelineEvent| event.sender().to_string())
        .unwrap_or_default();
    // A decrypted event carries the encryption info; an event this device could
    // not decrypt is still an encrypted event.
    let is_encrypted = target.encryption_info().is_some() || undecryptable;

    let (relations, relations_truncated) = relation_page(&room, &event_id).await?;

    let edit = latest_valid_edit(&target, &relations);
    let edited = edit.map(|edit| DirectMessageEdit {
        event_id: edit.event_id().map(ToString::to_string).unwrap_or_default(),
        timestamp_ms: edit
            .timestamp()
            .map(|timestamp| u64::from(timestamp.0))
            .unwrap_or_default(),
    });
    let own_user_id = session.client().user_id().map(ToOwned::to_owned);
    let reactions = reaction_groups(&relations, own_user_id.as_deref());

    // The SDK's own projection: an edit event becomes its `m.new_content`, and
    // a reply's fallback is stripped. An event this device cannot decrypt has no
    // content, which is a successful read with no body.
    let displayed = if is_redacted || message.is_none() {
        None
    } else {
        let source = edit.cloned().unwrap_or_else(|| target.clone());
        matrix_sdk_ui::timeline::TimelineItemContent::from_event(&room, source).await
    };
    let body = displayed
        .as_ref()
        .and_then(|content| content.as_message())
        .map(|message| {
            // The timeline's own rule: a reply's quoted fallback is not display
            // text, so a summary does not repeat the quoted message.
            let mut msgtype = message.msgtype().clone();
            let remove = if reply_to_event_id.is_some() {
                RemoveReplyFallback::Yes
            } else {
                RemoveReplyFallback::No
            };
            msgtype.sanitize(HtmlSanitizerMode::Compat, remove);
            msgtype.body().to_owned()
        })
        .filter(|body| !body.is_empty());

    Ok(DirectMessageView {
        room_id: room_id.to_owned(),
        event_id: event_id.to_string(),
        sender_id,
        timestamp_ms,
        kind,
        is_encrypted,
        is_redacted,
        content_available: body.is_some(),
        body,
        edited,
        reactions,
        relations_truncated,
        thread_root_event_id,
        reply_to_event_id,
    })
}

/// Reads a target and the relations page an action has to decide on.
async fn target_and_relations(
    session: &MatrixClientSession,
    room_id: &str,
    event_id: &str,
) -> Result<
    (
        matrix_sdk::Room,
        matrix_sdk::ruma::OwnedEventId,
        TimelineEvent,
        SyncMessageLikeEvent<RoomMessageEventContent>,
        Vec<TimelineEvent>,
        bool,
    ),
    DirectMessageFailure,
> {
    let room = joined_room(session, room_id)?;
    let event_id = parse_event_id(event_id)?;
    let target = load_event_authoritative(&room, &event_id).await?;
    let message = message_of(&target).ok_or(DirectMessageFailure::NotAMessage)?;
    let (relations, truncated) = relation_page(&room, &event_id).await?;
    Ok((room, event_id, target, message, relations, truncated))
}

fn own_user_id(
    session: &MatrixClientSession,
) -> Result<matrix_sdk::ruma::OwnedUserId, DirectMessageFailure> {
    session
        .client()
        .user_id()
        .map(ToOwned::to_owned)
        .ok_or(DirectMessageFailure::AccountInactive)
}

/// Adds this account's reaction with `key`, with set semantics.
///
/// The caller serializes the decision per (room, target, key), so two calls
/// cannot both decide that no own reaction exists.
pub(crate) async fn react_direct(
    session: Arc<MatrixClientSession>,
    room_id: &str,
    event_id: &str,
    key: &str,
) -> Result<DirectReactionOutcome, DirectMessageFailure> {
    let own = own_user_id(&session)?;
    let (room, target_id, _target, message, relations, truncated) =
        target_and_relations(&session, room_id, event_id).await?;
    let (_, is_redacted) = kind_and_redaction(&message);
    if is_redacted {
        return Err(DirectMessageFailure::Redacted);
    }
    let (existing, unresolved) = own_reaction_event_ids(&relations, &own, key);
    if !existing.is_empty() {
        return Ok(DirectReactionOutcome {
            added: false,
            reaction_event_id: existing.first().map(ToString::to_string),
        });
    }
    if truncated || unresolved {
        // Without the whole set this cannot prove the reaction is absent, and a
        // guess would add a duplicate.
        return Err(DirectMessageFailure::RelationsUnavailable);
    }
    let content = matrix_sdk::ruma::events::reaction::ReactionEventContent::new(
        matrix_sdk::ruma::events::relation::Annotation::new(target_id, key.to_owned()),
    );
    let response = crate::executor::timeout(READ_TIMEOUT, async { room.send(content).await })
        .await
        .map_err(|_| DirectMessageFailure::Unavailable)?
        .map_err(|_| DirectMessageFailure::Unavailable)?;
    Ok(DirectReactionOutcome {
        added: true,
        reaction_event_id: Some(response.response.event_id.to_string()),
    })
}

/// Removes every own reaction with `key`; zero is not an error.
pub(crate) async fn unreact_direct(
    session: Arc<MatrixClientSession>,
    room_id: &str,
    event_id: &str,
    key: &str,
) -> Result<u32, DirectMessageFailure> {
    let own = own_user_id(&session)?;
    let (room, _target_id, _target, message, relations, truncated) =
        target_and_relations(&session, room_id, event_id).await?;
    let (_, is_redacted) = kind_and_redaction(&message);
    if is_redacted {
        return Err(DirectMessageFailure::Redacted);
    }
    let (ids, unresolved) = own_reaction_event_ids(&relations, &own, key);
    if ids.is_empty() {
        if truncated || unresolved {
            return Err(DirectMessageFailure::RelationsUnavailable);
        }
        return Ok(0);
    }
    if truncated || unresolved {
        // Removing the visible subset would report success while other own
        // reactions with this key remain.
        return Err(DirectMessageFailure::RelationsUnavailable);
    }
    for id in &ids {
        crate::executor::timeout(READ_TIMEOUT, room.redact(id, None, None))
            .await
            .map_err(|_| DirectMessageFailure::Unavailable)?
            .map_err(|_| DirectMessageFailure::Unavailable)?;
    }
    Ok(ids.len() as u32)
}

/// Replaces this account's own message text, keeping its message type.
pub(crate) async fn edit_direct(
    session: Arc<MatrixClientSession>,
    room_id: &str,
    event_id: &str,
    body: &str,
    formatting: koushi_state::ComposerFormattingOptions,
) -> Result<DirectEditOutcome, DirectMessageFailure> {
    let own = own_user_id(&session)?;
    let (room, target_id, _target, message, _relations, _truncated) =
        target_and_relations(&session, room_id, event_id).await?;
    let sender = message.sender();
    if sender != own {
        return Err(DirectMessageFailure::NotAuthor);
    }
    let SyncMessageLikeEvent::Original(original) = &message else {
        return Err(DirectMessageFailure::Redacted);
    };
    if original
        .content
        .relates_to
        .as_ref()
        .is_some_and(|relation| matches!(relation, Relation::Replacement(_)))
    {
        return Err(DirectMessageFailure::NotAMessage);
    }
    let document = koushi_state::ComposerDocument::from_plain_text(body.to_owned());
    let content = super::item_projection::edited_document_content_for_edit_target(
        Some(&original.content.msgtype),
        &document,
        formatting,
    );
    let edit_event = room
        .make_edit_event(&target_id, content)
        .await
        .map_err(|_| DirectMessageFailure::Unavailable)?;
    let response = crate::executor::timeout(READ_TIMEOUT, async {
        room.send(edit_event).require_backed_up_session().await
    })
    .await
    .map_err(|_| DirectMessageFailure::Unavailable)?
    .map_err(|_| DirectMessageFailure::Unavailable)?;
    Ok(DirectEditOutcome {
        edit_event_id: Some(response.response.event_id.to_string()),
    })
}

/// Redacts this account's own message; an already-redacted target is a success.
pub(crate) async fn delete_direct(
    session: Arc<MatrixClientSession>,
    room_id: &str,
    event_id: &str,
) -> Result<DirectRedactOutcome, DirectMessageFailure> {
    let own = own_user_id(&session)?;
    let room = joined_room(&session, room_id)?;
    let target_id = parse_event_id(event_id)?;
    let target = load_event_authoritative(&room, &target_id).await?;
    let message = message_of(&target).ok_or(DirectMessageFailure::NotAMessage)?;
    if message.sender() != own {
        return Err(DirectMessageFailure::NotAuthor);
    }
    let (_, is_redacted) = kind_and_redaction(&message);
    if is_redacted {
        return Ok(DirectRedactOutcome {
            redaction_event_id: None,
            already_redacted: true,
        });
    }
    let redaction = crate::executor::timeout(READ_TIMEOUT, room.redact(&target_id, None, None))
        .await
        .map_err(|_| DirectMessageFailure::Unavailable)?
        .map_err(|_| DirectMessageFailure::Unavailable)?;
    Ok(DirectRedactOutcome {
        redaction_event_id: Some(redaction.event_id.to_string()),
        already_redacted: false,
    })
}

#[cfg(test)]
#[path = "direct_message_tests.rs"]
mod tests;

/// Posts a plain-text reply under a visible thread root (#1628).
///
/// The content is the composer's own thread shape (`make_offline_thread_content`
/// over the composer document builder) and the insertion is the SDK room send
/// queue, but the completion is observed on this call's own channel. Nothing is
/// registered as a pending send, no resident actor is refreshed, and no
/// `CoreEvent` is emitted, so an open thread pane cannot change and the GUI sees
/// the result only through the normal sync echo.
pub(crate) async fn thread_reply_direct(
    session: Arc<MatrixClientSession>,
    room_id: &str,
    root_event_id: &str,
    body: &str,
    formatting: koushi_state::ComposerFormattingOptions,
) -> Result<crate::direct_operations::DirectThreadReplyOutcome, DirectMessageFailure> {
    const SEND_TIMEOUT: Duration = Duration::from_secs(60);
    let room = joined_room(&session, room_id)?;
    let root_id = parse_event_id(root_event_id)?;
    let root = load_event_authoritative(&room, &root_id).await?;
    let message = message_of(&root).ok_or(DirectMessageFailure::NotAMessage)?;
    let (_, is_redacted) = kind_and_redaction(&message);
    if is_redacted {
        return Err(DirectMessageFailure::Redacted);
    }
    if relations_of(&message).0.is_some() {
        return Err(DirectMessageFailure::NotThreadRoot);
    }

    let key = koushi_protocol::ids::TimelineKey {
        account_key: crate::store::account_key_from_info(&session.info),
        kind: koushi_protocol::ids::TimelineKind::Thread {
            room_id: room_id.to_owned(),
            root_event_id: root_event_id.to_owned(),
        },
    };
    let document = koushi_state::ComposerDocument::from_plain_text(body.to_owned());
    let mut content =
        super::composer::build_room_message_content_from_composer_document_with_options(
            document, formatting,
        )
        .map_err(|_| DirectMessageFailure::NotAMessage)?;
    super::outbound_send::make_offline_thread_content(&mut content, &key)
        .map_err(|_| DirectMessageFailure::Unavailable)?;

    let queue = room.send_queue();
    // Subscribe before enqueuing so the terminal update cannot be missed.
    let (_echoes, mut updates) = queue
        .subscribe()
        .await
        .map_err(|_| DirectMessageFailure::Unavailable)?;
    let handle = queue
        .send(content.into())
        .await
        .map_err(|_| DirectMessageFailure::Unavailable)?;
    let transaction_id = handle.transaction_id().to_owned();

    let outcome = crate::executor::timeout(SEND_TIMEOUT, async {
        loop {
            match updates.recv().await {
                Ok(matrix_sdk::send_queue::RoomSendQueueUpdate::SentEvent {
                    transaction_id: sent,
                    event_id,
                }) if sent == transaction_id => return Some(event_id),
                Ok(matrix_sdk::send_queue::RoomSendQueueUpdate::SendError {
                    transaction_id: failed,
                    ..
                }) if failed == transaction_id => return None,
                Ok(matrix_sdk::send_queue::RoomSendQueueUpdate::CancelledLocalEvent {
                    transaction_id: cancelled,
                }) if cancelled == transaction_id => return None,
                Ok(_) => continue,
                Err(_) => return None,
            }
        }
    })
    .await
    .map_err(|_| DirectMessageFailure::Unavailable)?;

    let Some(event_id) = outcome else {
        // The queue keeps a failed request; it is not resent by this caller.
        return Err(DirectMessageFailure::Unavailable);
    };
    Ok(crate::direct_operations::DirectThreadReplyOutcome {
        event_id: event_id.to_string(),
        transaction_id: transaction_id.to_string(),
    })
}

/// Runs one direct message request (#1628).
///
/// The caller keeps the future boxed, because the operation-wide deadline wraps
/// a large future and the account actor's loop must stay small.
pub(crate) async fn run_direct_message_action(
    session: Arc<MatrixClientSession>,
    reaction_lane: Arc<tokio::sync::Mutex<()>>,
    request: crate::direct_operations::DirectMessageRequest,
) -> Result<
    crate::direct_operations::DirectMessageResponse,
    crate::direct_operations::DirectMessageFailure,
> {
    use crate::direct_operations::{
        DirectMessageRequest as Request, DirectMessageResponse as Response,
    };
    match request {
        Request::Read { room_id, event_id } => read_direct_message(session, &room_id, &event_id)
            .await
            .map(Response::Read),
        Request::React {
            room_id,
            event_id,
            key,
        } => {
            let _serialized = reaction_lane.lock().await;
            react_direct(session, &room_id, &event_id, &key)
                .await
                .map(Response::React)
        }
        Request::Unreact {
            room_id,
            event_id,
            key,
        } => {
            let _serialized = reaction_lane.lock().await;
            unreact_direct(session, &room_id, &event_id, &key)
                .await
                .map(Response::Unreact)
        }
        Request::Edit {
            room_id,
            event_id,
            body,
            formatting,
        } => edit_direct(session, &room_id, &event_id, &body, formatting)
            .await
            .map(Response::Edit),
        Request::Delete { room_id, event_id } => delete_direct(session, &room_id, &event_id)
            .await
            .map(Response::Delete),
        Request::ThreadReply {
            room_id,
            root_event_id,
            body,
            formatting,
        } => thread_reply_direct(session, &room_id, &root_event_id, &body, formatting)
            .await
            .map(Response::ThreadReply),
    }
}
