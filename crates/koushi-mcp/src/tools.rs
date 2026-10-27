//! Tool definitions and argument validation.
//!
//! Schemas live here, not in the app, so `tools/list` still answers when Koushi
//! is not running. The app validates the same arguments again before dispatch;
//! this layer only stops malformed calls early and gives the model a stable
//! description.

use serde_json::{Value, json};

/// One MCP tool as advertised to the client.
#[derive(Clone, Debug)]
pub struct ToolDefinition {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
    pub read_only: bool,
    /// The `outputSchema` of a tool whose result is typed structured content.
    pub output_schema: Option<Value>,
}

fn account_id_property() -> Value {
    json!({
        "type": "string",
        "description": "Stable account handle from list_accounts. Required; never inferred from the visible tab."
    })
}

fn limit_property(maximum: u32) -> Value {
    json!({
        "type": "integer",
        "minimum": 1,
        "maximum": maximum,
        "description": "Maximum rows to return."
    })
}

/// The structured result of `wait_for_events` (#1551).
fn wait_for_events_output_schema() -> Value {
    let nullable_string = json!({ "type": ["string", "null"] });
    json!({
        "type": "object",
        "properties": {
            "events": {
                "type": "array",
                "description": "One entry per sampled room change after the cursor, oldest first.",
                "items": {
                    "type": "object",
                    "properties": {
                        "cursor": { "type": "string", "description": "Opaque position of this entry." },
                        "account_id": { "type": "string" },
                        "room_id": { "type": "string" },
                        "room_name": { "type": "string" },
                        "event_id": nullable_string,
                        "sender_id": nullable_string,
                        "sender_label": nullable_string,
                        "unread_count": { "type": "integer", "description": "The room's unread count when sampled; informational." },
                        "highlight_count": { "type": "integer" },
                        "recency_stamp": { "type": ["integer", "null"] }
                    },
                    "required": ["cursor", "account_id", "room_id", "event_id"]
                }
            },
            "cursor": { "type": "string", "description": "Opaque position to pass to the next call." },
            "coverage": {
                "type": "string",
                "enum": ["sampled_room_changes"],
                "description": "Always sampled_room_changes: entries coalesce bursts and are not a complete message log."
            }
        },
        "required": ["events", "cursor", "coverage"]
    })
}

fn room_event_properties() -> Value {
    json!({
        "account_id": account_id_property(),
        "room_id": { "type": "string", "description": "Room id of a room this account is joined to." },
        "event_id": { "type": "string", "description": "Event id of a message-like event in that room." }
    })
}

fn client_request_id_property() -> Value {
    json!({
        "type": "string",
        "description": "Caller-generated UUID. Reuse it verbatim when retrying so the action is not applied twice."
    })
}

fn reaction_key_property() -> Value {
    json!({
        "type": "string",
        "minLength": 1,
        "description": "Emoji or short text of the reaction, 1-128 UTF-8 bytes with no control characters or line breaks."
    })
}

fn id_or_null() -> Value {
    json!({ "type": ["string", "null"] })
}

/// The structured result of `get_message` (#1628).
fn get_message_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "room_id": { "type": "string" },
            "event_id": { "type": "string" },
            "sender_id": { "type": "string" },
            "timestamp_ms": { "type": "integer" },
            "kind": { "type": "string" },
            "is_encrypted": { "type": "boolean" },
            "is_redacted": { "type": "boolean" },
            "content_available": { "type": "boolean", "description": "False when this device cannot decrypt the event; body is then null and the read still succeeded." },
            "body": { "type": ["string", "null"], "description": "Display body after the latest valid edit, with any reply fallback removed." },
            "edited": {
                "type": ["object", "null"],
                "properties": {
                    "event_id": { "type": "string" },
                    "timestamp_ms": { "type": "integer" }
                },
                "required": ["event_id", "timestamp_ms"]
            },
            "reactions": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "key": { "type": "string" },
                        "count": { "type": "integer" },
                        "mine": { "type": "boolean" }
                    },
                    "required": ["key", "count", "mine"]
                }
            },
            "relations_truncated": { "type": "boolean", "description": "True when more relations exist than this bounded read returned." },
            "thread_root_event_id": id_or_null(),
            "reply_to_event_id": id_or_null()
        },
        "required": [
            "room_id", "event_id", "sender_id", "timestamp_ms", "kind", "is_encrypted",
            "is_redacted", "content_available", "body", "edited", "reactions",
            "relations_truncated", "thread_root_event_id", "reply_to_event_id"
        ]
    })
}

fn message_action_tool(
    name: &'static str,
    description: &'static str,
    input_schema: Value,
    output_schema: Value,
) -> ToolDefinition {
    ToolDefinition {
        name,
        description,
        input_schema,
        read_only: false,
        output_schema: Some(output_schema),
    }
}

/// Every tool this build advertises. Order is stable so `tools/list` diffs are
/// meaningful across runs.
pub fn definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "list_accounts",
            description: "List the signed-in accounts this connection may use. Returns a stable account_id, display name, user id, and availability.",
            input_schema: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
            read_only: true,
            output_schema: None,
        },
        ToolDefinition {
            name: "list_rooms",
            description: "List rooms and direct messages the account can see, with encryption state, display name, unread counts, and the latest event's sender and timestamp. unread_only restricts the list to rooms with something unread. Requires account_id.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "account_id": account_id_property(),
                    "limit": limit_property(200),
                    "cursor": { "type": "string", "description": "Opaque cursor from the previous page." },
                    "unread_only": { "type": "boolean", "description": "Return only rooms with unread messages, notifications, mentions, or a manual unread mark. Defaults to false." }
                },
                "required": ["account_id"],
                "additionalProperties": false
            }),
            read_only: true,
            output_schema: None,
        },
        ToolDefinition {
            name: "search_messages",
            description: "Search decrypted message history through Koushi's existing encrypted index. Results carry account provenance, room id, event id, sender, timestamp, and a snippet. Requires account_id.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "account_id": account_id_property(),
                    "query": { "type": "string", "minLength": 1, "description": "Search text." },
                    "room_ids": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Restrict to these rooms. The connection grant narrows this further."
                    },
                    "limit": limit_property(100)
                },
                "required": ["account_id", "query"],
                "additionalProperties": false
            }),
            read_only: true,
            output_schema: None,
        },
        ToolDefinition {
            name: "read_room_messages",
            description: "Read a room's most recent messages, newest first, with sender, timestamp, and text. Works for a room nobody has opened and never changes the GUI. Pass next from a previous page as from to read older messages. Requires account_id and room_id.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "account_id": account_id_property(),
                    "room_id": { "type": "string" },
                    "limit": limit_property(100),
                    "from": { "type": "string", "description": "Opaque token from the previous page's next value." }
                },
                "required": ["account_id", "room_id"],
                "additionalProperties": false
            }),
            read_only: true,
            output_schema: None,
        },
        ToolDefinition {
            name: "get_attachment",
            description: "Download one attachment of a message into Koushi's managed media cache and return its path, mime type, and size. Koushi chooses the destination. Requires account_id, room_id, and event_id.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "account_id": account_id_property(),
                    "room_id": { "type": "string" },
                    "event_id": { "type": "string" },
                    "thumbnail": { "type": "boolean", "description": "Return a thumbnail instead of the original file. Defaults to false." }
                },
                "required": ["account_id", "room_id", "event_id"],
                "additionalProperties": false
            }),
            read_only: true,
            output_schema: None,
        },
        ToolDefinition {
            name: "send_message",
            description: "Send a message through the account's verified encrypted session. body is plain text. attachments are absolute local file paths (at most 10, 50 MB each, 100 MB per call) that Koushi reads and sends as one media message per file, with body as the first file's caption; every file is checked and read before the first is sent. Sending files needs the account's attachments permission as well as send. reply_to_event_id cannot be combined with attachments. Returns event_id and transaction_id of the first message and, when files were sent, an attachments list with one event_id and transaction_id per file. client_request_id makes a retry idempotent.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "account_id": account_id_property(),
                    "room_id": { "type": "string" },
                    "body": { "type": "string", "description": "Plain text body. Required unless attachments are present; with attachments it captions the first file." },
                    "reply_to_event_id": { "type": "string", "description": "Event to reply to. Not allowed together with attachments." },
                    "attachments": {
                        "type": "array",
                        "maxItems": 10,
                        "description": "Local files to send, each as its own media message, in this order.",
                        "items": {
                            "type": "object",
                            "properties": { "path": { "type": "string", "description": "Absolute path of a regular file readable by Koushi." } },
                            "required": ["path"],
                            "additionalProperties": false
                        }
                    },
                    "client_request_id": {
                        "type": "string",
                        "description": "Caller-generated UUID. Reuse it verbatim when retrying so the send is not duplicated."
                    }
                },
                "required": ["account_id", "room_id", "client_request_id"],
                "additionalProperties": false
            }),
            read_only: false,
            output_schema: None,
        },
        ToolDefinition {
            name: "read_recent_activity",
            description: "Read account-wide Activity Recent, including already-read items, without opening Activity or changing GUI navigation, unread counts or receipts. This is bounded observed history, not a complete archive: at most 200 existing Activity rows, muted/low-priority exclusions and notification filters apply. No background history resolver is started. Newest first using Core ordering. Unknown read state is explicit. Default limit 50, maximum 100. Opaque immutable snapshot cursors expire after 120 seconds, restart, account/session/caller/query/access changes or bounded-cache eviction; repeat all query fields when continuing. resync_required means start a fresh read without cursor and use read_room_messages for available surrounding history, deduplicating account/room/event IDs. Exhaustion or an empty result does not imply no messages occurred.",
            input_schema: json!({
                "type":"object", "properties":{
                    "account_id": account_id_property(), "limit":limit_property(100),
                    "room_ids":{"type":"array","items":{"type":"string"},"maxItems":64},
                    "from_timestamp_ms":{"type":"integer","minimum":0,"description":"Inclusive timestamp boundary."},
                    "to_timestamp_ms":{"type":"integer","minimum":0,"description":"Exclusive timestamp boundary."},
                    "cursor":{"type":"string","description":"Opaque continuation; repeat the same filters and limit."}
                },"required":["account_id"],"additionalProperties":false
            }),
            read_only: true,
            output_schema: Some(json!({"type":"object", "properties":{
                "account_id":{"type":"string"}, "items":{"type":"array", "items":{"type":"object", "properties":{
                    "account_id":{"type":"string"},"room_id":{"type":"string"},
                    "event_id":{"type":["string","null"]},"sender_id":{"type":["string","null"]},
                    "thread_root_event_id":{"type":["string","null"]},"timestamp_ms":{"type":"integer"},
                    "preview":{"type":["string","null"]},"content_available":{"type":"boolean"},
                    "preview_truncated":{"type":"boolean"},"metadata_truncated":{"type":"boolean"},"read_state":{"const":"unknown"}
                },"required":["account_id","room_id","event_id","sender_id","timestamp_ms","preview","read_state","content_available","preview_truncated"]}},
                "cursor":{"type":["string","null"]},"snapshot_timestamp_ms":{"type":"integer"},
                "coverage":{"const":"bounded_observed_recent"},"history_complete":{"const":false},
                "retention_limit":{"const":200},"truncated":{"type":"boolean"},"expires_after_s":{"const":120}
            },"required":["account_id","items","cursor","snapshot_timestamp_ms","coverage","history_complete","truncated","expires_after_s","retention_limit"]})),
        },
        ToolDefinition {
            name: "wait_for_events",
            description: "Wait up to timeout_s for sampled room changes in the account. This is a change feed, not a message log: Koushi samples each joined room's latest event about every 2 seconds and records one entry per room whose latest event changed since the previous sample, so several messages posted between samples coalesce into one entry. Invites are not reported. The first sample after Koushi or the bridge starts only sets a baseline. Entries are ordered by cursor, kept in memory only (at most 500 across all accounts), and are not removed when a room is read; unread counts are informational. Returns at once when entries are pending, otherwise at the timeout. An empty events list means no change was sampled, not that no message was posted. Omit cursor to start at the newest position (this does not recover earlier history); pass the returned cursor to continue. A cursor is opaque and valid only for this account and this Koushi run: a malformed, future, expired (older than the retained entries), other-account, or previous-run cursor fails with resync_required. To recover, call again without a cursor and use read_room_messages for the rooms you need, de-duplicating by account, room, and event id.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "account_id": account_id_property(),
                    "cursor": { "type": "string", "description": "Opaque cursor from the previous result. Omit to start at the newest position." },
                    "timeout_s": { "type": "integer", "minimum": 0, "maximum": 50, "description": "Seconds to wait when nothing is pending. Defaults to 25; an approval wait counts against a 60-second call budget." }
                },
                "required": ["account_id"],
                "additionalProperties": false
            }),
            read_only: true,
            output_schema: Some(wait_for_events_output_schema()),
        },
        ToolDefinition {
            name: "get_message",
            description: "Fetch one message-like event of a joined room by id, whether or not the room is open in the GUI. Returns the sender, timestamp, kind, the body after its latest valid edit, reaction groups, and the thread/reply relation. Never marks anything read, sends a receipt, changes unread state, or starts a background resolver. An event this device cannot decrypt is a successful read with content_available false and body null. Reactions and edits come from at most one bounded relations page, so relations_truncated reports that more exist. Requires account_id, room_id, and event_id.",
            input_schema: json!({
                "type": "object",
                "properties": room_event_properties(),
                "required": ["account_id", "room_id", "event_id"],
                "additionalProperties": false
            }),
            read_only: true,
            output_schema: Some(get_message_output_schema()),
        },
        message_action_tool(
            "react_to_message",
            "Add this account's reaction with the given key to a message-like event of a joined room, without opening the room or changing any GUI state. Set semantics: if the account already reacted with that key, nothing is sent and added is false with the existing reaction event id. The result is the user's own reaction, visible to others. A reaction that cannot be resolved from one bounded relations page fails instead of risking a duplicate. Requires the message actions permission.",
            json!({
                "type": "object",
                "properties": {
                    "account_id": account_id_property(),
                    "room_id": { "type": "string" },
                    "event_id": { "type": "string" },
                    "key": reaction_key_property()
                },
                "required": ["account_id", "room_id", "event_id", "key"],
                "additionalProperties": false
            }),
            json!({
                "type": "object",
                "properties": {
                    "added": { "type": "boolean" },
                    "reaction_event_id": id_or_null()
                },
                "required": ["added", "reaction_event_id"]
            }),
        ),
        message_action_tool(
            "unreact_to_message",
            "Remove every reaction this account added to a message-like event with the given key, without opening the room or changing any GUI state. Sending no reaction with that key is not an error: removed is 0. Requires the message actions permission.",
            json!({
                "type": "object",
                "properties": {
                    "account_id": account_id_property(),
                    "room_id": { "type": "string" },
                    "event_id": { "type": "string" },
                    "key": reaction_key_property()
                },
                "required": ["account_id", "room_id", "event_id", "key"],
                "additionalProperties": false
            }),
            json!({
                "type": "object",
                "properties": { "removed": { "type": "integer", "minimum": 0 } },
                "required": ["removed"]
            }),
        ),
        message_action_tool(
            "edit_message",
            "Replace the display text of a message this account sent, keeping its message type and any media payload (editing an image or file caption does not turn it into a text event). Fails with not_permitted when the target belongs to another sender. Reusing the same client_request_id does not create a second edit event. A direct edit is sent as one event without the send queue, so transaction_id echoes client_request_id for correlation rather than naming a queued transaction. Requires the message actions permission.",
            json!({
                "type": "object",
                "properties": {
                    "account_id": account_id_property(),
                    "room_id": { "type": "string" },
                    "event_id": { "type": "string" },
                    "body": { "type": "string", "minLength": 1, "description": "New plain-text body." },
                    "client_request_id": client_request_id_property()
                },
                "required": ["account_id", "room_id", "event_id", "body", "client_request_id"],
                "additionalProperties": false
            }),
            json!({
                "type": "object",
                "properties": {
                    "edit_event_id": id_or_null(),
                    "transaction_id": { "type": "string", "description": "The caller's client_request_id, echoed for correlation; a direct edit has no queued SDK transaction." }
                },
                "required": ["edit_event_id", "transaction_id"]
            }),
        ),
        message_action_tool(
            "delete_message",
            "Redact a message this account sent, without opening the room or changing any GUI state. Moderation of another user's message is not supported in this version. Deleting an already-redacted message succeeds with already_redacted true and sends no new event. Requires the message actions permission.",
            json!({
                "type": "object",
                "properties": room_event_properties(),
                "required": ["account_id", "room_id", "event_id"],
                "additionalProperties": false
            }),
            json!({
                "type": "object",
                "properties": {
                    "redaction_event_id": id_or_null(),
                    "already_redacted": { "type": "boolean" }
                },
                "required": ["redaction_event_id", "already_redacted"]
            }),
        ),
        ToolDefinition {
            name: "send_thread_reply",
            description: "Post a plain-text reply in the thread under a visible thread root, using the same thread relation and reply fallback the composer sends, without opening the thread or changing any GUI state. The target must be a thread root, not itself a thread reply; it may belong to another user. Attachments are not supported here. Reusing the same client_request_id does not post twice. Needs the send permission. If the caller disconnects before the answer, the reply may already be posted and cannot be recalled.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "account_id": account_id_property(),
                    "room_id": { "type": "string" },
                    "thread_root_event_id": { "type": "string", "description": "Event id of the thread root to answer in." },
                    "body": { "type": "string", "minLength": 1, "description": "Plain text body." },
                    "client_request_id": client_request_id_property()
                },
                "required": ["account_id", "room_id", "thread_root_event_id", "body", "client_request_id"],
                "additionalProperties": false
            }),
            read_only: false,
            output_schema: Some(json!({
                "type": "object",
                "properties": {
                    "event_id": { "type": "string" },
                    "transaction_id": { "type": "string" }
                },
                "required": ["event_id", "transaction_id"]
            })),
        },
    ]
}

/// Looks a tool up by name.
pub fn find(name: &str) -> Option<ToolDefinition> {
    definitions().into_iter().find(|tool| tool.name == name)
}

/// Validates the shape of a `tools/call` argument object.
///
/// This is deliberately shallow: required properties, scalar types, and the
/// account-id rule. Semantic checks (membership, grant, account state) belong
/// to the app, which owns that state.
pub fn validate_arguments(name: &str, arguments: &Value) -> Result<(), String> {
    let Some(tool) = find(name) else {
        return Err(format!("unknown tool: {name}"));
    };
    let Value::Object(map) = arguments else {
        return Err("arguments must be an object".to_owned());
    };
    let schema = &tool.input_schema;
    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for key in required {
            let Some(key) = key.as_str() else { continue };
            if !map.contains_key(key) {
                return Err(format!("missing required argument: {key}"));
            }
        }
    }
    if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
        for (key, value) in map {
            let Some(property) = properties.get(key) else {
                return Err(format!("unknown argument: {key}"));
            };
            let expected = property.get("type").and_then(Value::as_str);
            let matches = match expected {
                Some("string") => value.is_string(),
                Some("integer") => value.is_i64() || value.is_u64(),
                Some("array") => value.is_array(),
                Some("object") => value.is_object(),
                Some("boolean") => value.is_boolean(),
                _ => true,
            };
            if !matches {
                return Err(format!("argument {key} has the wrong type"));
            }
        }
    }
    if map.contains_key("account_id")
        && map
            .get("account_id")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
    {
        return Err("account_id must be a non-empty string".to_owned());
    }
    Ok(())
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
