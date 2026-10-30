//! Tool handlers. Every handler returns a JSON value; the caller wraps it in an
//! MCP tool result.
//!
//! Every account-scoped tool first resolves the account to a live runtime, then
//! asks the consent broker for the permission it needs. The broker re-reads
//! `mcp/connections.json` on every call, so a revocation applies at once, and
//! asks the user in Koushi when the permission is missing.

use serde_json::{Value, json};
use std::sync::Arc;

use crate::BridgeAccounts;
use crate::consent::{ClientIdentity, ConsentContext};
use crate::server::Shared;

use koushi_mcp::wire::{ErrorCode, WireError};
use koushi_protocol::command::{CoreCommand, TimelineCommand};
use koushi_protocol::event::{CoreEvent, TimelineEvent};
use koushi_protocol::ids::TimelineKey;
use koushi_state::{ComposerDocument, SessionInfo, SessionState};

use super::events;
use super::grants::Need;

/// One `tools/call` request.
pub(crate) async fn dispatch_tool_call(
    shared: &Shared,
    client: &ClientIdentity,
    params: Value,
) -> Result<Value, WireError> {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return Err(WireError::new(
            ErrorCode::Unsupported,
            "tools/call requires a tool name",
        ));
    };
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    let result = match name {
        "list_accounts" => list_accounts(shared, client),
        "list_rooms" => list_rooms(shared, client, &arguments).await,
        "search_messages" => search_messages(shared, client, &arguments).await,
        "send_message" => send_message(shared, client, &arguments).await,
        "wait_for_events" => wait_for_events(shared, client, &arguments).await,
        "read_recent_activity" => read_recent_activity(shared, client, &arguments).await,
        "get_attachment" => get_attachment(shared, client, &arguments).await,
        "read_room_messages" => read_room_messages(shared, client, &arguments).await,
        "get_message" => get_message(shared, client, &arguments).await,
        "react_to_message" => react_to_message(shared, client, &arguments).await,
        "unreact_to_message" => unreact_to_message(shared, client, &arguments).await,
        "edit_message" => edit_message(shared, client, &arguments).await,
        "delete_message" => delete_message(shared, client, &arguments).await,
        "send_thread_reply" => send_thread_reply(shared, client, &arguments).await,
        "draft_message" => draft_message(shared, client, &arguments).await,
        other => Err(WireError::new(
            ErrorCode::Unsupported,
            format!("unknown tool: {other}"),
        )),
    };

    Ok(tool_result(result))
}

/// An MCP tool result. The text content keeps compatibility with clients that
/// read only text; `structuredContent` carries the same object, or the typed
/// error (`code`, optional stable `reason`, `message`), so a client can tell a
/// quiet result from a failure without parsing prose (#1551).
pub(crate) fn tool_result(result: Result<Value, WireError>) -> Value {
    match result {
        Ok(value) => json!({
            "content": [{ "type": "text", "text": value.to_string() }],
            "structuredContent": value,
            "isError": false
        }),
        Err(error) => {
            let mut detail = json!({
                "code": error.code.as_str(),
                "message": error.message,
            });
            if let Some(reason) = &error.reason {
                detail["reason"] = json!(reason);
            }
            let text = format!("{}: {}", error.code.as_str(), error.message);
            json!({
                "content": [{ "type": "text", "text": text }],
                "structuredContent": { "error": detail },
                "isError": true
            })
        }
    }
}

/// Every account is listed so a first-time client can ask for access. The
/// account id itself embeds the Matrix user ID (the canon states this
/// disclosure); display fields appear only once the client holds some
/// permission for the account.
/// `permissions` says which; a missing one is asked for in Koushi on first use.
fn list_accounts(shared: &Shared, client: &ClientIdentity) -> Result<Value, WireError> {
    let store = shared.broker.store();
    let mut listed = Vec::new();
    for account in shared.accounts.accounts() {
        let snapshot = account.connection.snapshot();
        let (status, session) = session_status(&snapshot.session);
        let grant = store.account(&client.name, &account.id);
        let permissions = json!({
            "read": grant.is_some_and(|grant| grant.read),
            "send": grant.is_some_and(|grant| grant.send),
            "attachments": grant.is_some_and(|grant| grant.attachments),
            "message_actions": grant.is_some_and(|grant| grant.message_actions),
            "draft": grant.is_some_and(|grant| grant.draft),
        });
        if grant.is_some_and(|grant| {
            grant.read || grant.send || grant.attachments || grant.message_actions || grant.draft
        }) {
            let user_id = session.map(|info| info.user_id.clone());
            let homeserver = session
                .map(|info| info.homeserver.clone())
                .or_else(|| account.homeserver.clone());
            listed.push(json!({
                "account_id": account.id,
                "user_id": user_id,
                "homeserver": homeserver,
                "display_name": session.map(display_name_from_user_id),
                "status": status,
                "consent": "granted",
                "permissions": permissions,
            }));
        } else {
            listed.push(json!({
                "account_id": account.id,
                "status": status,
                "consent": "required",
                "permissions": permissions,
            }));
        }
    }
    Ok(json!({ "accounts": listed }))
}

/// Resolves the account, then checks or asks for `need`. An unknown account
/// fails before any approval is shown.
async fn authorize(
    shared: &Shared,
    client: &ClientIdentity,
    account_id: &str,
    need: Need,
    context: ConsentContext,
) -> Result<koushi_core::CoreConnection, WireError> {
    let connection = account_connection(&shared.accounts, account_id)?;
    shared
        .broker
        .authorize(client, account_id, need, context)
        .await?;
    // The account can be signed out or locked while the approval waited.
    ready_account_key(&connection.snapshot().session)?;
    Ok(connection)
}

fn session_status(session: &SessionState) -> (&'static str, Option<&SessionInfo>) {
    match session {
        SessionState::Ready(info) => ("ready", Some(info)),
        SessionState::Locked(info) => ("locked", Some(info)),
        SessionState::AwaitingVerification { info, .. } => ("verification_pending", Some(info)),
        SessionState::Provisional { info, .. } => ("provisional", Some(info)),
        SessionState::SignedOut => ("signed_out", None),
        _ => ("unavailable", None),
    }
}

fn display_name_from_user_id(info: &SessionInfo) -> String {
    info.user_id
        .strip_prefix('@')
        .and_then(|rest| rest.split(':').next())
        .unwrap_or(&info.user_id)
        .to_owned()
}

async fn list_rooms(
    shared: &Shared,
    client: &ClientIdentity,
    arguments: &Value,
) -> Result<Value, WireError> {
    let account_id = required_account_id(arguments)?;
    let connection = authorize(
        shared,
        client,
        &account_id,
        Need::Read,
        ConsentContext::default(),
    )
    .await?;
    let limit = arguments
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(50)
        .clamp(1, 200) as usize;
    let offset = arguments
        .get("cursor")
        .and_then(Value::as_str)
        .and_then(|cursor| cursor.parse::<usize>().ok())
        .unwrap_or(0);

    let unread_only = arguments
        .get("unread_only")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let snapshot = connection.snapshot();
    // The rooms of this very snapshot: a sign-out or lock while the approval
    // waited must not read as an account with no rooms (#1580).
    ready_account_key(&snapshot.session)?;
    let mut rooms: Vec<Value> = snapshot
        .rooms
        .iter()
        .filter(|room| {
            !unread_only
                || room.unread_count > 0
                || room.notification_count > 0
                || room.highlight_count > 0
                || room.thread_unread_count > 0
                || room.marked_unread
        })
        .map(|room| {
            json!({
                "room_id": room.room_id,
                "name": room.display_label,
                "is_dm": room.is_dm,
                "is_encrypted": room.is_encrypted,
                "unread_messages": room.unread_count,
                "unread_notifications": room.notification_count,
                "unread_mentions": room.highlight_count,
                "thread_unread_messages": room.thread_unread_count,
                "thread_unread_mentions": room.thread_highlight_count,
                "marked_unread": room.marked_unread,
                "recency_stamp": room.recency_stamp,
                "last_event": room.latest_event.as_ref().map(|event| json!({
                    "event_id": event.event_id,
                    "sender_id": event.sender_id,
                    "timestamp_ms": event.timestamp_ms,
                    "preview": event.preview,
                })),
            })
        })
        .collect();

    let total = rooms.len();
    let page: Vec<Value> = rooms.drain(..).skip(offset).take(limit).collect();
    let next = offset + page.len();
    Ok(json!({
        "rooms": page,
        "next_cursor": (next < total).then(|| next.to_string()),
    }))
}

fn direct_search_failure(failure: koushi_core::search::DirectSearchFailure) -> WireError {
    use koushi_core::search::DirectSearchFailure as Failure;
    let (code, message) = match failure {
        Failure::AccountInactive => (ErrorCode::AccountUnavailable, "account is not signed in"),
        Failure::Capacity => (
            ErrorCode::Busy,
            "too many concurrent searches for this account",
        ),
        Failure::ForbiddenRoom => (ErrorCode::RoomNotFound, "a requested room is not joined"),
        Failure::Search(koushi_protocol::failure::SearchFailureKind::IndexUnavailable) => {
            (ErrorCode::Internal, "the search index is unavailable")
        }
        Failure::Search(_) => (ErrorCode::Internal, "search failed"),
    };
    WireError::new(code, message)
}

async fn search_messages(
    shared: &Shared,
    client: &ClientIdentity,
    arguments: &Value,
) -> Result<Value, WireError> {
    let account_id = required_account_id(arguments)?;
    let connection = authorize(
        shared,
        client,
        &account_id,
        Need::Read,
        ConsentContext::default(),
    )
    .await?;
    let operation = connection.begin_navigation_operation(
        koushi_diagnostics::NavigationCause::DirectSearch,
        koushi_diagnostics::NavigationOrigin::Mcp,
    );
    let result = async {
        let query = arguments
            .get("query")
            .and_then(Value::as_str)
            .ok_or_else(|| WireError::new(ErrorCode::Unsupported, "query is required"))?
            .trim()
            .to_owned();
        if query.is_empty() {
            return Err(WireError::new(ErrorCode::Unsupported, "query is empty"));
        }
        let limit = arguments
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(20)
            .clamp(1, 100) as usize;
        let room_ids: Vec<String> = arguments
            .get("room_ids")
            .and_then(Value::as_array)
            .map(|rooms| {
                rooms
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();

        let room_names: std::collections::HashMap<String, String> = connection
            .snapshot()
            .rooms
            .iter()
            .map(|room| (room.room_id.clone(), room.display_label.clone()))
            .collect();

        let results = connection
            .direct_search(query, room_ids)
            .await
            .map_err(direct_search_failure)?;
        // The account can be signed out or locked while the search ran.
        ready_account_key(&connection.snapshot().session)?;

        let total = results.len();
        let items: Vec<Value> = results
            .into_iter()
            .take(limit)
            .map(|item| {
                let room_name = room_names
                    .get(&item.room_id)
                    .cloned()
                    .unwrap_or_else(|| item.room_id.clone());
                json!({
                    "room_id": item.room_id,
                    "room_name": room_name,
                    "event_id": item.event_id,
                    "snippet": item.snippet,
                    "sender_id": item.sender_id,
                    "timestamp_ms": item.timestamp_ms,
                })
            })
            .collect();
        Ok(json!({ "results": items, "total": total }))
    }
    .await;
    operation.finish(result.is_ok());
    result
}

/// Reads one page of a room's recent messages for the agent (#1530).
///
/// Koushi reads it from the account session, so a room nobody opened still
/// answers and the GUI's open room, scroll position, and pagination stay put.
async fn read_room_messages(
    shared: &Shared,
    client: &ClientIdentity,
    arguments: &Value,
) -> Result<Value, WireError> {
    let account_id = required_account_id(arguments)?;
    let connection = authorize(
        shared,
        client,
        &account_id,
        Need::Read,
        ConsentContext::default(),
    )
    .await?;
    let operation = connection.begin_navigation_operation(
        koushi_diagnostics::NavigationCause::DirectRead,
        koushi_diagnostics::NavigationOrigin::Mcp,
    );
    let result = async {
        let room_id = arguments
            .get("room_id")
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| WireError::new(ErrorCode::Unsupported, "room_id is required"))?
            .to_owned();
        let limit = arguments
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(30)
            .clamp(1, 100) as u32;
        let from = arguments
            .get("from")
            .and_then(Value::as_str)
            .filter(|token| !token.is_empty())
            .map(str::to_owned);

        let page = connection
            .read_room_messages(room_id.clone(), limit, from)
            .await
            .map_err(room_messages_failure)?;
        // The account can be signed out or locked while the read ran.
        ready_account_key(&connection.snapshot().session)?;
        let messages: Vec<Value> = page
            .messages
            .into_iter()
            .map(|message| {
                json!({
                    "event_id": message.event_id,
                    "sender_id": message.sender_id,
                    "timestamp_ms": message.timestamp_ms,
                    "body": message.body,
                    "kind": message.kind,
                    "is_encrypted": message.is_encrypted,
                    "is_redacted": message.is_redacted,
                })
            })
            .collect();
        Ok(json!({
            "room_id": room_id,
            "messages": messages,
            "next": page.next,
        }))
    }
    .await;
    operation.finish(result.is_ok());
    result
}

/// Maps one direct message failure to the closed MCP error vocabulary (#1628).
fn direct_message_failure(failure: koushi_core::DirectMessageFailure) -> WireError {
    use koushi_core::DirectMessageFailure as Failure;
    match failure {
        Failure::AccountInactive => {
            WireError::new(ErrorCode::AccountUnavailable, "account is not signed in")
        }
        Failure::RoomNotFound => WireError::new(ErrorCode::RoomNotFound, "the room is not joined"),
        Failure::EventNotFound => {
            WireError::new(ErrorCode::EventNotFound, "the event is not in this room")
        }
        Failure::NotAuthor => WireError::with_reason(
            ErrorCode::NotPermitted,
            "not_author",
            "only the message's own sender can change it",
        ),
        Failure::NotAMessage => WireError::with_reason(
            ErrorCode::Unsupported,
            "not_a_message",
            "the event is not a message",
        ),
        Failure::Redacted => WireError::with_reason(
            ErrorCode::Unsupported,
            "redacted",
            "the message was already deleted",
        ),
        Failure::NotThreadRoot => WireError::with_reason(
            ErrorCode::Unsupported,
            "not_thread_root",
            "the event is a thread reply, not a thread root",
        ),
        Failure::RelationsUnavailable => WireError::with_reason(
            ErrorCode::Internal,
            "relations_unavailable",
            "Koushi could not read this message's reactions",
        ),
        Failure::Capacity => WireError::new(
            ErrorCode::Busy,
            "too many concurrent message actions for this account",
        ),
        Failure::DuplicatePending => WireError::with_reason(
            ErrorCode::Busy,
            "duplicate_pending",
            "a request with this client_request_id is still running or its outcome is unknown",
        ),
        Failure::Unavailable => {
            WireError::new(ErrorCode::Internal, "Koushi could not complete this action")
        }
    }
}

fn required_argument<'a>(arguments: &'a Value, name: &str) -> Result<&'a str, WireError> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| WireError::new(ErrorCode::Unsupported, format!("{name} is required")))
}

/// A reaction key is 1-128 UTF-8 bytes with no control characters or line
/// breaks, so it cannot smuggle a multi-line body into a reaction.
fn reaction_key(arguments: &Value) -> Result<&str, WireError> {
    let key = required_argument(arguments, "key")?;
    let invalid = key.len() > 128 || key.chars().any(|character| character.is_control());
    if invalid {
        return Err(WireError::new(
            ErrorCode::Unsupported,
            "a reaction key is 1-128 bytes with no control characters",
        ));
    }
    Ok(key)
}

/// A caller-generated id used as the retry key. It becomes a transaction id
/// label, so it is restricted to the characters those ids use.
fn client_request_id(arguments: &Value) -> Result<&str, WireError> {
    let id = required_argument(arguments, "client_request_id")?;
    let valid = id.len() <= 128
        && id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-._:".contains(character));
    if !valid {
        return Err(WireError::new(
            ErrorCode::Unsupported,
            "client_request_id must be at most 128 characters of ASCII letters, digits, - . _ or :",
        ));
    }
    Ok(id)
}

/// Reads one message-like event by id for an out-of-band caller (#1628).
async fn get_message(
    shared: &Shared,
    client: &ClientIdentity,
    arguments: &Value,
) -> Result<Value, WireError> {
    let account_id = required_account_id(arguments)?;
    let room_id = required_argument(arguments, "room_id")?.to_owned();
    let event_id = required_argument(arguments, "event_id")?.to_owned();
    let connection = authorize(
        shared,
        client,
        &account_id,
        Need::Read,
        ConsentContext::default(),
    )
    .await?;
    let operation = connection.begin_navigation_operation(
        koushi_diagnostics::NavigationCause::DirectRead,
        koushi_diagnostics::NavigationOrigin::Mcp,
    );
    let result = async {
        let view = match connection
            .direct_message_action(koushi_core::DirectMessageRequest::Read {
                room_id: room_id.clone(),
                event_id: event_id.clone(),
            })
            .await
            .map_err(direct_message_failure)?
        {
            koushi_core::DirectMessageResponse::Read(view) => view,
            _ => {
                return Err(WireError::new(
                    ErrorCode::Internal,
                    "Koushi answered a different action",
                ));
            }
        };
        // The account can be signed out or locked while the read ran.
        ready_account_key(&connection.snapshot().session)?;
        Ok(json!({
            "room_id": view.room_id,
            "event_id": view.event_id,
            "sender_id": view.sender_id,
            "timestamp_ms": view.timestamp_ms,
            "kind": view.kind,
            "is_encrypted": view.is_encrypted,
            "is_redacted": view.is_redacted,
            "content_available": view.content_available,
            "body": view.body,
            "edited": view.edited.map(|edit| json!({
                "event_id": edit.event_id,
                "timestamp_ms": edit.timestamp_ms,
            })),
            "reactions": view.reactions.into_iter().map(|reaction| json!({
                "key": reaction.key,
                "count": reaction.count,
                "mine": reaction.mine,
            })).collect::<Vec<_>>(),
            "relations_truncated": view.relations_truncated,
            "thread_root_event_id": view.thread_root_event_id,
            "reply_to_event_id": view.reply_to_event_id,
        }))
    }
    .await;
    operation.finish(result.is_ok());
    result
}

/// Authorizes `message_actions` and returns the account connection (#1628).
///
/// The approval is raised before any target detail is read, so an approval can
/// be shown for a call that then fails. Every target verdict (own author,
/// message kind, redacted, thread root) belongs to Core, which reads the target
/// inside the operation immediately before it submits.
async fn authorize_message_action(
    shared: &Shared,
    client: &ClientIdentity,
    account_id: &str,
    room_id: &str,
) -> Result<koushi_core::CoreConnection, WireError> {
    authorize(
        shared,
        client,
        account_id,
        Need::MessageActions,
        ConsentContext {
            room_label: joined_room_label(
                &account_connection(&shared.accounts, account_id)?,
                room_id,
            ),
            message_preview: None,
        },
    )
    .await
}

/// Adds this account's reaction with set semantics (#1628).
async fn react_to_message(
    shared: &Shared,
    client: &ClientIdentity,
    arguments: &Value,
) -> Result<Value, WireError> {
    let account_id = required_account_id(arguments)?;
    let room_id = required_argument(arguments, "room_id")?.to_owned();
    let event_id = required_argument(arguments, "event_id")?.to_owned();
    let key = reaction_key(arguments)?.to_owned();
    let connection = authorize_message_action(shared, client, &account_id, &room_id).await?;
    match connection
        .direct_message_action(koushi_core::DirectMessageRequest::React {
            room_id,
            event_id,
            key,
        })
        .await
        .map_err(direct_message_failure)?
    {
        koushi_core::DirectMessageResponse::React(outcome) => Ok(json!({
            "added": outcome.added,
            "reaction_event_id": outcome.reaction_event_id,
        })),
        _ => Err(WireError::new(
            ErrorCode::Internal,
            "Koushi answered a different action",
        )),
    }
}

/// Removes every own reaction with `key` (#1628).
async fn unreact_to_message(
    shared: &Shared,
    client: &ClientIdentity,
    arguments: &Value,
) -> Result<Value, WireError> {
    let account_id = required_account_id(arguments)?;
    let room_id = required_argument(arguments, "room_id")?.to_owned();
    let event_id = required_argument(arguments, "event_id")?.to_owned();
    let key = reaction_key(arguments)?.to_owned();
    let connection = authorize_message_action(shared, client, &account_id, &room_id).await?;
    match connection
        .direct_message_action(koushi_core::DirectMessageRequest::Unreact {
            room_id,
            event_id,
            key,
        })
        .await
        .map_err(direct_message_failure)?
    {
        koushi_core::DirectMessageResponse::Unreact(removed) => Ok(json!({ "removed": removed })),
        _ => Err(WireError::new(
            ErrorCode::Internal,
            "Koushi answered a different action",
        )),
    }
}

/// Replaces this account's own message text (#1628).
async fn edit_message(
    shared: &Shared,
    client: &ClientIdentity,
    arguments: &Value,
) -> Result<Value, WireError> {
    let account_id = required_account_id(arguments)?;
    let room_id = required_argument(arguments, "room_id")?.to_owned();
    let event_id = required_argument(arguments, "event_id")?.to_owned();
    let body = required_argument(arguments, "body")?.to_owned();
    let request_id = client_request_id(arguments)?.to_owned();
    // Core's retry ledger answers a repeated key; every call is authorized
    // first, so a repeat never answers after the grant or the session went away.
    let connection = authorize_message_action(shared, client, &account_id, &room_id).await?;
    let formatting = connection
        .snapshot()
        .settings
        .values
        .composer
        .formatting_options();
    match connection
        .direct_message_action(koushi_core::DirectMessageRequest::Edit {
            room_id,
            event_id,
            body,
            formatting,
            retry: Some(koushi_core::DirectRetryKey {
                client: client.name.clone(),
                client_request_id: request_id.clone(),
            }),
        })
        .await
        .map_err(direct_message_failure)?
    {
        koushi_core::DirectMessageResponse::Edit(outcome) => Ok(json!({
            "edit_event_id": outcome.edit_event_id,
            // A direct edit sends one event without the queue, so there is
            // no SDK transaction id; the caller's retry key is echoed here
            // for correlation.
            "transaction_id": request_id,
        })),
        _ => Err(WireError::new(
            ErrorCode::Internal,
            "Koushi answered a different action",
        )),
    }
}

/// Redacts this account's own message (#1628).
async fn delete_message(
    shared: &Shared,
    client: &ClientIdentity,
    arguments: &Value,
) -> Result<Value, WireError> {
    let account_id = required_account_id(arguments)?;
    let room_id = required_argument(arguments, "room_id")?.to_owned();
    let event_id = required_argument(arguments, "event_id")?.to_owned();
    let connection = authorize_message_action(shared, client, &account_id, &room_id).await?;
    match connection
        .direct_message_action(koushi_core::DirectMessageRequest::Delete { room_id, event_id })
        .await
        .map_err(direct_message_failure)?
    {
        koushi_core::DirectMessageResponse::Delete(outcome) => Ok(json!({
            "redaction_event_id": outcome.redaction_event_id,
            "already_redacted": outcome.already_redacted,
        })),
        _ => Err(WireError::new(
            ErrorCode::Internal,
            "Koushi answered a different action",
        )),
    }
}

/// Posts a plain-text reply under a visible thread root (#1628).
///
/// The reply reuses the composer's thread content shape and the direct send
/// queue that `send_message` already uses, so no thread timeline is mounted.
async fn send_thread_reply(
    shared: &Shared,
    client: &ClientIdentity,
    arguments: &Value,
) -> Result<Value, WireError> {
    let account_id = required_account_id(arguments)?;
    let room_id = required_argument(arguments, "room_id")?.to_owned();
    let root_event_id = required_argument(arguments, "thread_root_event_id")?.to_owned();
    let body = required_argument(arguments, "body")?.to_owned();
    let request_id = client_request_id(arguments)?.to_owned();
    // Core's retry ledger answers a repeated key at the submission below; every
    // call is authorized first, so a repeat never answers after the grant or
    // the session went away.
    let connection = account_connection(&shared.accounts, &account_id)?;
    let room_label = joined_room_label(&connection, &room_id);
    shared
        .broker
        .authorize(
            client,
            &account_id,
            Need::Send,
            ConsentContext {
                room_label,
                message_preview: (!body.trim().is_empty()).then(|| body.clone()),
            },
        )
        .await?;
    ready_account_key(&connection.snapshot().session)?;
    let operation = connection.begin_navigation_operation(
        koushi_diagnostics::NavigationCause::DirectSend,
        koushi_diagnostics::NavigationOrigin::Mcp,
    );
    let result = async {
        // The target must be a visible message-like thread root, not itself a
        // thread reply. The read is bounded by the same budget as `get_message`.
        let view = match connection
            .direct_message_action(koushi_core::DirectMessageRequest::Read {
                room_id: room_id.clone(),
                event_id: root_event_id.clone(),
            })
            .await
            .map_err(direct_message_failure)?
        {
            koushi_core::DirectMessageResponse::Read(view) => view,
            _ => {
                return Err(WireError::new(
                    ErrorCode::Internal,
                    "Koushi answered a different action",
                ));
            }
        };
        if view.is_redacted {
            return Err(direct_message_failure(
                koushi_core::DirectMessageFailure::Redacted,
            ));
        }
        if view.thread_root_event_id.is_some() {
            return Err(direct_message_failure(
                koushi_core::DirectMessageFailure::NotThreadRoot,
            ));
        }
        // The grant is re-read immediately before the send, so a revocation
        // during the read is honored.
        shared
            .broker
            .authorize(client, &account_id, Need::Send, ConsentContext::default())
            .await?;
        let snapshot = connection.snapshot();
        ready_account_key(&snapshot.session)?;
        let formatting = snapshot.settings.values.composer.formatting_options();
        match connection
            .direct_message_action(koushi_core::DirectMessageRequest::ThreadReply {
                room_id: room_id.clone(),
                root_event_id: root_event_id.clone(),
                body,
                formatting,
                retry: Some(koushi_core::DirectRetryKey {
                    client: client.name.clone(),
                    client_request_id: request_id,
                }),
            })
            .await
            .map_err(direct_message_failure)?
        {
            koushi_core::DirectMessageResponse::ThreadReply(outcome) => Ok(json!({
                "event_id": outcome.event_id,
                "transaction_id": outcome.transaction_id,
            })),
            _ => Err(WireError::new(
                ErrorCode::Internal,
                "Koushi answered a different action",
            )),
        }
    }
    .await;
    operation.finish(result.is_ok());
    result
}

/// Stages plain text in a room's or a thread's composer without sending it
/// (#1644).
///
/// The draft goes through Core's Rust draft store, the only composer write an
/// MCP call may make. It never selects a room, account, pane, or thread, and
/// it never overwrites a draft: a non-empty or busy composer answers
/// `conflict`.
async fn draft_message(
    shared: &Shared,
    client: &ClientIdentity,
    arguments: &Value,
) -> Result<Value, WireError> {
    let account_id = required_account_id(arguments)?;
    let room_id = required_argument(arguments, "room_id")?.to_owned();
    let body = arguments
        .get("body")
        .and_then(Value::as_str)
        .filter(|body| !body.trim().is_empty())
        .ok_or_else(|| WireError::new(ErrorCode::Unsupported, "body is empty"))?
        .to_owned();
    if body.len() > koushi_state::MAX_PERSISTED_COMPOSER_DRAFT_BYTES {
        return Err(WireError::with_reason(
            ErrorCode::Unsupported,
            "body_too_long",
            "body is longer than a composer draft can hold (16 KiB)",
        ));
    }
    if arguments.get("reply_to_event_id").is_some() || arguments.get("attachments").is_some() {
        return Err(WireError::new(
            ErrorCode::Unsupported,
            "draft_message stages plain text only; replies and attachments are not supported",
        ));
    }
    let thread_root_event_id = arguments
        .get("thread_root_event_id")
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .map(str::to_owned);

    // As for a send, the approval previews the text, and the room only when
    // this account has joined it; whether it exists is decided after consent.
    let connection = account_connection(&shared.accounts, &account_id)?;
    shared
        .broker
        .authorize(
            client,
            &account_id,
            Need::Draft,
            ConsentContext {
                room_label: joined_room_label(&connection, &room_id),
                message_preview: Some(body.clone()),
            },
        )
        .await?;
    ready_account_key(&connection.snapshot().session)?;
    if joined_room_label(&connection, &room_id).is_none() {
        return Err(WireError::new(
            ErrorCode::RoomNotFound,
            "the room is not joined",
        ));
    }

    let target = match &thread_root_event_id {
        None => koushi_state::ComposerTarget::Main {
            room_id: room_id.clone(),
        },
        Some(root_event_id) => {
            // The same root rule as `send_thread_reply`: a visible message that
            // is not itself a thread reply.
            let view = match connection
                .direct_message_action(koushi_core::DirectMessageRequest::Read {
                    room_id: room_id.clone(),
                    event_id: root_event_id.clone(),
                })
                .await
                .map_err(direct_message_failure)?
            {
                koushi_core::DirectMessageResponse::Read(view) => view,
                _ => {
                    return Err(WireError::new(
                        ErrorCode::Internal,
                        "Koushi answered a different action",
                    ));
                }
            };
            if view.is_redacted {
                return Err(direct_message_failure(
                    koushi_core::DirectMessageFailure::Redacted,
                ));
            }
            if view.thread_root_event_id.is_some() {
                return Err(direct_message_failure(
                    koushi_core::DirectMessageFailure::NotThreadRoot,
                ));
            }
            // A revocation during the read is honored before the write.
            shared
                .broker
                .authorize(client, &account_id, Need::Draft, ConsentContext::default())
                .await?;
            koushi_state::ComposerTarget::Thread {
                room_id: room_id.clone(),
                root_event_id: root_event_id.clone(),
            }
        }
    };

    connection
        .stage_assistant_draft(koushi_core::AssistantDraftRequest {
            target,
            document: ComposerDocument::from_plain_text(body),
            client_name: client.name.clone(),
        })
        .await
        .map_err(assistant_draft_failure)?;
    Ok(json!({
        "room_id": room_id,
        "thread_root_event_id": thread_root_event_id,
        "status": "staged",
    }))
}

fn assistant_draft_failure(failure: koushi_core::AssistantDraftFailure) -> WireError {
    use koushi_core::AssistantDraftFailure as Failure;
    match failure {
        Failure::AccountInactive => {
            WireError::new(ErrorCode::AccountUnavailable, "account is not signed in")
        }
        Failure::RoomNotFound => WireError::new(ErrorCode::RoomNotFound, "the room is not joined"),
        Failure::Conflict => WireError::new(
            ErrorCode::Conflict,
            "the composer already holds a draft or is in use; nothing was changed",
        ),
        Failure::Unavailable => WireError::with_reason(
            ErrorCode::Busy,
            "drafts_loading",
            "Koushi has not loaded this account's drafts yet; try again shortly",
        ),
    }
}

fn room_messages_failure(failure: koushi_core::RoomMessagesFailure) -> WireError {
    use koushi_core::RoomMessagesFailure as Failure;
    let (code, message) = match failure {
        Failure::AccountInactive => (ErrorCode::AccountUnavailable, "account is not signed in"),
        Failure::RoomNotFound => (ErrorCode::RoomNotFound, "the room is not joined"),
        Failure::Unavailable => (
            ErrorCode::Internal,
            "Koushi could not read this room's messages",
        ),
    };
    WireError::new(code, message)
}

async fn send_message(
    shared: &Shared,
    client: &ClientIdentity,
    arguments: &Value,
) -> Result<Value, WireError> {
    let account_id = required_account_id(arguments)?;
    let room_id = arguments
        .get("room_id")
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| WireError::new(ErrorCode::Unsupported, "room_id is required"))?
        .to_owned();
    let body = arguments
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let attachments = attachment_paths(arguments)?;
    if attachments.is_empty() && body.trim().is_empty() {
        return Err(WireError::new(
            ErrorCode::Unsupported,
            "body is empty and no attachments were given",
        ));
    }
    let in_reply_to = arguments
        .get("reply_to_event_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_owned);
    if in_reply_to.is_some() && !attachments.is_empty() {
        return Err(WireError::new(
            ErrorCode::Unsupported,
            "reply_to_event_id cannot be combined with attachments; send the reply and the files separately",
        ));
    }
    let client_request_id = arguments
        .get("client_request_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty());
    // A call that can never succeed (relative path, missing file, over the
    // limits) fails before it raises an approval. Contents are read only after
    // consent.
    if !attachments.is_empty() {
        koushi_core::local_upload::check_local_paths(&attachments).map_err(local_upload_failure)?;
    }

    // The approval shows the message as context, and the room only when it is
    // one this account has joined. Whether the room exists is decided after
    // consent, so an unapproved client cannot probe membership.
    let mut connection = account_connection(&shared.accounts, &account_id)?;
    let room_label = joined_room_label(&connection, &room_id);
    shared
        .broker
        .authorize(
            client,
            &account_id,
            Need::Send,
            ConsentContext {
                room_label: room_label.clone(),
                message_preview: (!body.trim().is_empty()).then(|| body.clone()),
            },
        )
        .await?;
    // Before the attachments permission can be asked: the account is checked
    // again after the approval wait, so a signed-out or locked account never
    // raises a second dialog.
    ready_account_key(&connection.snapshot().session)?;
    let operation = connection.begin_navigation_operation(
        koushi_diagnostics::NavigationCause::DirectSend,
        koushi_diagnostics::NavigationOrigin::Mcp,
    );
    let result = async {
        // Sending local files is file access: it also needs the attachments
        // permission, asked separately like every other permission.
        if !attachments.is_empty() {
            shared
                .broker
                .authorize(
                    client,
                    &account_id,
                    Need::Attachments,
                    ConsentContext {
                        room_label,
                        message_preview: None,
                    },
                )
                .await?;
        }
        let snapshot = connection.snapshot();
        // The account can be signed out or locked while the approval waited.
        let account_key = ready_account_key(&snapshot.session)?;
        let key = TimelineKey::room(account_key, room_id.clone());
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
        if !attachments.is_empty() {
            return Box::pin(send_attachments(
                &mut connection,
                &snapshot,
                AttachmentSend {
                    key,
                    room_id,
                    body,
                    paths: attachments,
                    client_request_id,
                    deadline,
                },
            ))
            .await;
        }

        let request_id = connection.next_request_id();
        // Caller-supplied ids make a retried send idempotent at the core.
        let transaction_id = client_request_id
            .map(|id| format!("mcp-{id}"))
            .unwrap_or_else(|| format!("mcp-{}", request_id.sequence));
        let document = ComposerDocument::from_plain_text(body);
        let command = match in_reply_to {
            Some(event_id) => TimelineCommand::SendReply {
                request_id,
                key: key.clone(),
                transaction_id: transaction_id.clone(),
                in_reply_to_event_id: event_id,
                document,
            },
            None => TimelineCommand::SendText {
                request_id,
                key: key.clone(),
                transaction_id: transaction_id.clone(),
                document,
            },
        };
        connection
            .command(CoreCommand::Timeline(command))
            .await
            .map_err(|_| WireError::new(ErrorCode::Internal, "send could not be submitted"))?;
        let (transaction_id, event_id) =
            wait_send_completed(&mut connection, request_id, &key, deadline).await?;
        Ok(json!({
            "room_id": room_id,
            "event_id": event_id,
            "transaction_id": transaction_id,
        }))
    }
    .await;
    operation.finish(result.is_ok());
    result
}

/// `attachments[].path`, in order. An absent or empty array means none.
fn attachment_paths(arguments: &Value) -> Result<Vec<std::path::PathBuf>, WireError> {
    let Some(list) = arguments.get("attachments") else {
        return Ok(Vec::new());
    };
    let list = list
        .as_array()
        .ok_or_else(|| WireError::new(ErrorCode::Unsupported, "attachments must be an array"))?;
    list.iter()
        .map(|item| {
            item.get("path")
                .and_then(Value::as_str)
                .filter(|path| !path.is_empty())
                .map(std::path::PathBuf::from)
                .ok_or_else(|| {
                    WireError::new(
                        ErrorCode::Unsupported,
                        "every attachment needs a non-empty path",
                    )
                })
        })
        .collect()
}

fn local_upload_failure(failure: koushi_core::local_upload::LocalUploadFailure) -> WireError {
    use koushi_core::local_upload::LocalUploadFailure as Failure;
    let (code, message) = match failure {
        Failure::NoFiles => (ErrorCode::Unsupported, "no attachments were given"),
        Failure::TooManyFiles => (
            ErrorCode::Unsupported,
            "at most 10 attachments can be sent per call",
        ),
        Failure::NotAbsolute => (ErrorCode::Unsupported, "attachment paths must be absolute"),
        Failure::NotARegularFile => (
            ErrorCode::Unsupported,
            "an attachment path is not a regular file",
        ),
        Failure::Unreadable => (ErrorCode::Unsupported, "an attachment file cannot be read"),
        Failure::TooLarge => (
            ErrorCode::AttachmentTooLarge,
            "attachments are limited to 50 MB per file and 100 MB per call",
        ),
        Failure::Empty => (ErrorCode::Unsupported, "an attachment file is empty"),
        Failure::PreparationFailed => (
            ErrorCode::Internal,
            "Koushi could not prepare an attachment for upload",
        ),
    };
    WireError::new(code, message)
}

/// One attachment send request, after consent.
struct AttachmentSend<'a> {
    key: TimelineKey,
    room_id: String,
    body: String,
    paths: Vec<std::path::PathBuf>,
    client_request_id: Option<&'a str>,
    deadline: tokio::time::Instant,
}

/// Sends each local file as its own media message through the Core upload
/// command the composer's staged uploads end in, without touching composer
/// state. Every file is read and prepared before the first one is sent, so a
/// bad file sends nothing; a delivery failure part-way says how many were
/// already sent.
async fn send_attachments(
    connection: &mut koushi_core::CoreConnection,
    snapshot: &koushi_state::AppState,
    send: AttachmentSend<'_>,
) -> Result<Value, WireError> {
    let AttachmentSend {
        key,
        room_id,
        body,
        paths,
        client_request_id,
        deadline,
    } = send;
    let SessionState::Ready(info) = &snapshot.session else {
        return Err(WireError::new(
            ErrorCode::AccountUnavailable,
            "account is not ready to send",
        ));
    };
    let expected_account = koushi_core::store::session_key_id_from_info(info);
    let caption = (!body.trim().is_empty()).then(|| ComposerDocument::from_plain_text(body));
    let requests = koushi_core::local_upload::prepare_local_uploads(
        paths,
        caption,
        snapshot.settings.values.composer.formatting_options(),
        snapshot
            .settings
            .values
            .media
            .image_upload_compression_policy,
        None,
    )
    .await
    .map_err(local_upload_failure)?;
    let total = requests.len();
    let base = client_request_id
        .map(|id| format!("mcp-{id}"))
        .unwrap_or_else(|| format!("mcp-{}", connection.next_request_id().sequence));
    let mut sent = Vec::with_capacity(total);
    for (index, request) in requests.into_iter().enumerate() {
        let request_id = connection.next_request_id();
        let submitted = connection
            .command(CoreCommand::Timeline(TimelineCommand::UploadAndSendMedia {
                request_id,
                expected_account: expected_account.clone(),
                key: key.clone(),
                transaction_id: format!("{base}-{index}"),
                request,
            }))
            .await
            .map_err(|_| WireError::new(ErrorCode::Internal, "send could not be submitted"));
        let outcome = match submitted {
            Ok(()) => wait_send_completed(connection, request_id, &key, deadline).await,
            Err(error) => Err(error),
        };
        match outcome {
            Ok((transaction_id, event_id)) => sent.push(json!({
                "event_id": event_id,
                "transaction_id": transaction_id,
            })),
            Err(error) if sent.is_empty() => return Err(error),
            Err(error) => {
                return Err(WireError::new(
                    error.code,
                    format!(
                        "{} after {} of {total} attachments were sent",
                        error.message,
                        sent.len()
                    ),
                ));
            }
        }
    }
    let first = sent.first().cloned().unwrap_or(Value::Null);
    Ok(json!({
        "room_id": room_id,
        "event_id": first["event_id"],
        "transaction_id": first["transaction_id"],
        "attachments": sent,
    }))
}

/// Waits for this request's `SendCompleted` and returns its transaction and
/// event ids, or the typed failure.
async fn wait_send_completed(
    connection: &mut koushi_core::CoreConnection,
    request_id: koushi_protocol::ids::RequestId,
    key: &TimelineKey,
    deadline: tokio::time::Instant,
) -> Result<(String, String), WireError> {
    loop {
        let event = tokio::time::timeout_at(deadline, connection.recv_event())
            .await
            .map_err(|_| WireError::new(ErrorCode::Internal, "send timed out"))?;
        match event {
            Ok(CoreEvent::Timeline(TimelineEvent::SendCompleted {
                request_id: completed,
                key: completed_key,
                transaction_id,
                event_id,
            })) if completed == request_id && &completed_key == key => {
                return Ok((transaction_id, event_id));
            }
            Ok(CoreEvent::OperationFailed {
                request_id: completed,
                failure,
            }) if completed == request_id => {
                return Err(WireError::new(
                    ErrorCode::Internal,
                    format!("send failed: {failure:?}"),
                ));
            }
            Ok(_) => continue,
            Err(_) => {
                return Err(WireError::new(
                    ErrorCode::Internal,
                    "event stream lagged during send",
                ));
            }
        }
    }
}

/// The whole `wait_for_events` call, approval wait included, stays under this
/// many seconds (Codex's default tool timeout; the launcher allows 70).
const WAIT_CALL_BUDGET_SECS: u64 = 60;

async fn read_recent_activity(
    shared: &Shared,
    client: &ClientIdentity,
    arguments: &Value,
) -> Result<Value, WireError> {
    let query = crate::recent_activity::Query::parse(arguments)?;
    let account_id = required_account_id(arguments)?;
    let connection = authorize(
        shared,
        client,
        &account_id,
        Need::Read,
        ConsentContext::default(),
    )
    .await?;
    let operation = connection.begin_navigation_operation(
        koushi_diagnostics::NavigationCause::DirectRead,
        koushi_diagnostics::NavigationOrigin::Mcp,
    );
    let result = async {
        let read = connection
            .read_recent_activity()
            .await
            .map_err(|_| WireError::new(ErrorCode::AccountUnavailable, "account is not ready"))?;
        let current = account_connection(&shared.accounts, &account_id)?;
        if !connection.shares_runtime_with(&current)
            || !shared
                .broker
                .store()
                .allows(&client.name, &account_id, Need::Read)
        {
            return Err(WireError::new(
                ErrorCode::AccountUnavailable,
                "account or authorization changed during the read",
            ));
        }
        let state = current.snapshot();
        if !read.matches_session(&state) {
            return Err(WireError::new(
                ErrorCode::AccountUnavailable,
                "session changed during the read",
            ));
        }
        if !read.matches_notification_modes(&state) {
            return Err(WireError::with_reason(
                ErrorCode::ResyncRequired,
                "filter_changed",
                "Activity filters changed during the read. Start a new read without cursor.",
            ));
        }
        let eligible = state
            .rooms
            .iter()
            .filter(|room| {
                room.tags.low_priority.is_none()
                    && !state
                        .room_notification_settings
                        .get(&room.room_id)
                        .is_some_and(|settings| {
                            settings.mode == koushi_state::RoomNotificationMode::Mute
                        })
            })
            .map(|room| room.room_id.clone())
            .collect();
        shared.activity_pages.read(
            &client.name,
            &account_id,
            query,
            arguments.get("cursor").and_then(Value::as_str),
            current,
            read,
            &eligible,
            &state.room_notification_settings,
        )
    }
    .await;
    operation.finish(result.is_ok());
    result
}

async fn wait_for_events(
    shared: &Shared,
    client: &ClientIdentity,
    arguments: &Value,
) -> Result<Value, WireError> {
    let started = tokio::time::Instant::now();
    let account_id = required_account_id(arguments)?;
    authorize(
        shared,
        client,
        &account_id,
        Need::Read,
        ConsentContext::default(),
    )
    .await?;
    let cursor = arguments.get("cursor").and_then(Value::as_str);
    let remaining = WAIT_CALL_BUDGET_SECS.saturating_sub(started.elapsed().as_secs());
    let timeout_s = arguments
        .get("timeout_s")
        .and_then(Value::as_u64)
        .unwrap_or(25)
        .min(remaining);

    let waited = events::wait_for_events(&shared.log, &account_id, cursor, timeout_s).await?;
    // The account can be signed out or locked while the wait ran: a quiet feed
    // of an account that is no longer usable is not an empty result (#1580).
    account_connection(&shared.accounts, &account_id)?;
    Ok(waited)
}

/// Downloads one file of a message into Koushi's managed media cache and
/// returns its absolute path. Koushi chooses the destination; the caller never
/// does.
async fn get_attachment(
    shared: &Shared,
    client: &ClientIdentity,
    arguments: &Value,
) -> Result<Value, WireError> {
    let account_id = required_account_id(arguments)?;
    let room_id = arguments
        .get("room_id")
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| WireError::new(ErrorCode::Unsupported, "room_id is required"))?
        .to_owned();
    let event_id = arguments
        .get("event_id")
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| WireError::new(ErrorCode::Unsupported, "event_id is required"))?
        .to_owned();
    let thumbnail = arguments
        .get("thumbnail")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let connection = account_connection(&shared.accounts, &account_id)?;
    let room_label = joined_room_label(&connection, &room_id);
    shared
        .broker
        .authorize(
            client,
            &account_id,
            Need::Attachments,
            ConsentContext {
                room_label,
                message_preview: None,
            },
        )
        .await?;
    let operation = connection.begin_navigation_operation(
        koushi_diagnostics::NavigationCause::DirectAttachment,
        koushi_diagnostics::NavigationOrigin::Mcp,
    );
    let result = async {
        let attachment = connection
            .fetch_attachment(room_id.clone(), event_id.clone(), thumbnail)
            .await
            .map_err(attachment_failure)?;
        ensure_cache_containment(&shared.accounts, &account_id, &attachment.path)?;
        // The account can be signed out or locked while the download ran.
        ready_account_key(&connection.snapshot().session)?;
        Ok(json!({
            "room_id": room_id,
            "event_id": event_id,
            "path": attachment.path.to_string_lossy(),
            "mime_type": attachment.mime_type,
            "size_bytes": attachment.size_bytes,
        }))
    }
    .await;
    operation.finish(result.is_ok());
    result
}

fn attachment_failure(failure: koushi_core::AttachmentFetchFailure) -> WireError {
    use koushi_core::AttachmentFetchFailure as Failure;
    let (code, message) = match failure {
        Failure::AccountInactive => (ErrorCode::AccountUnavailable, "account is not signed in"),
        Failure::RoomNotFound => (ErrorCode::RoomNotFound, "the room is not joined"),
        Failure::EventNotFound => (ErrorCode::EventNotFound, "the event is unknown to the room"),
        Failure::Unsupported => (ErrorCode::Unsupported, "the event carries no attachment"),
        Failure::DownloadFailed => (
            ErrorCode::Internal,
            "Koushi could not download this attachment",
        ),
    };
    WireError::new(code, message)
}

/// Refuse any returned path that leaves the account's media cache.
fn ensure_cache_containment(
    accounts: &Arc<dyn BridgeAccounts>,
    account_id: &str,
    path: &std::path::Path,
) -> Result<(), WireError> {
    let cache = accounts
        .accounts()
        .into_iter()
        .find(|account| account.id == account_id)
        .map(|account| account.media_cache_dir)
        .ok_or_else(|| WireError::new(ErrorCode::Internal, "unknown account cache"))?;
    let canonical_cache = cache
        .canonicalize()
        .map_err(|_| WireError::new(ErrorCode::Internal, "unknown account cache"))?;
    let canonical_path = path
        .canonicalize()
        .map_err(|_| WireError::new(ErrorCode::Internal, "attachment path is unavailable"))?;
    if canonical_path.starts_with(&canonical_cache) {
        Ok(())
    } else {
        Err(WireError::new(
            ErrorCode::Internal,
            "download finished outside Koushi's media cache",
        ))
    }
}

/// The room's label, only when the account has joined it. Used as approval
/// context; membership itself is enforced after consent by the tool.
fn joined_room_label(connection: &koushi_core::CoreConnection, room_id: &str) -> Option<String> {
    connection
        .snapshot()
        .rooms
        .iter()
        .find(|room| room.room_id == room_id)
        .map(|room| room.display_label.clone())
}

/// The account's live connection. A missing, signed-out, locked, or
/// still-restoring account fails the request before any approval exists, so an
/// unavailable account never reads as one with no rooms or no activity.
fn account_connection(
    accounts: &Arc<dyn BridgeAccounts>,
    account_id: &str,
) -> Result<koushi_core::CoreConnection, WireError> {
    let connection = accounts
        .accounts()
        .into_iter()
        .find(|account| account.id == account_id)
        .map(|account| account.connection)
        .ok_or_else(|| WireError::new(ErrorCode::AccountUnavailable, "unknown account"))?;
    ready_account_key(&connection.snapshot().session)?;
    Ok(connection)
}

fn required_account_id(arguments: &Value) -> Result<String, WireError> {
    arguments
        .get("account_id")
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| WireError::new(ErrorCode::Unsupported, "account_id is required"))
}

/// The signed-in account key. Only a ready session is a usable account: a
/// signed-out, locked, restoring, or still-verifying one answers
/// `account_unavailable`.
fn ready_account_key(
    session: &SessionState,
) -> Result<koushi_protocol::ids::AccountKey, WireError> {
    match session {
        SessionState::Ready(info) => Ok(koushi_core::store::account_key_from_info(info)),
        _ => Err(WireError::new(
            ErrorCode::AccountUnavailable,
            "account is not signed in",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A repeated `client_request_id` that Core's ledger holds is `busy` with a
    /// stable reason, distinct from the account's action cap (#1660).
    #[test]
    fn a_pending_retry_key_is_busy_with_its_own_reason() {
        let pending = direct_message_failure(koushi_core::DirectMessageFailure::DuplicatePending);
        assert_eq!(pending.code, ErrorCode::Busy);
        assert_eq!(pending.reason.as_deref(), Some("duplicate_pending"));
        let capacity = direct_message_failure(koushi_core::DirectMessageFailure::Capacity);
        assert_eq!(capacity.code, ErrorCode::Busy);
        assert_eq!(capacity.reason, None);
    }
}
