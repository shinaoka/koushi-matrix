//! `mcp_message_actions`: the display-independent message read/action entry
//! point an MCP caller uses (#1628), proven against a disposable homeserver.
//!
//! Account A posts synthetic messages in a room both accounts joined, then
//! drives every direct operation — read, set-semantics react/unreact, edit,
//! idempotent delete, thread reply — and account B, a separate account runtime,
//! re-reads the same events. B seeing the reaction, the edited body, the
//! redaction, and the thread relation is the proof that the effects reached the
//! homeserver instead of only A's local store.
//!
//! Nothing here asserts GUI state. The one timeline this scenario subscribes is
//! A's room timeline, and only because posting the fixture messages is a
//! timeline send; the operations run off the account session, and the scenario
//! never reads a timeline row, receipt, or unread value.

use super::cleanup::cleanup_logged_in_runtime;
use super::event_wait::{
    subscribe_timeline_for_qa, wait_for_invite_in_snapshot, wait_for_room_in_room_list,
    wait_for_send_flow_completion,
};
use super::fixtures::{accept_invite_for_qa, create_room_for_qa, invite_user_for_qa};
use super::participants::{
    QaParticipantLoginGate, QaParticipantLoginOutcome, login_synced_participant_for_qa, qa_data_dir,
};
use super::registry::QaConfig;
use super::{AccountKey, CoreCommand, CoreConnection, TimelineCommand, TimelineKey, TimelineKind};
use koushi_core::{
    DirectEditOutcome, DirectMessageRequest, DirectMessageResponse, DirectMessageView,
    DirectReactionOutcome, DirectRedactOutcome, DirectThreadReplyOutcome,
};
use koushi_protocol::event::RoomMessageKind;

const BODY: &str = "Synthetic MCP message actions body";
const EDITED_BODY: &str = "Synthetic MCP message actions edited body";
const DELETED_BODY: &str = "Synthetic MCP message actions deleted body";
const THREAD_ROOT_BODY: &str = "Synthetic MCP message actions thread root";
const THREAD_REPLY_BODY: &str = "Synthetic MCP message actions thread reply";
const REACTION_KEY: &str = "👍";

pub(super) async fn run_mcp_message_actions_scenario(config: &QaConfig) -> Result<(), String> {
    let QaParticipantLoginOutcome {
        runtime: runtime_a,
        conn: mut conn_a,
        account_key: account_key_a,
        ..
    } = login_synced_participant_for_qa(
        &config.homeserver,
        qa_data_dir("mcp-message-actions-a"),
        &config.user_a,
        &config.password_a,
        "Koushi MCP Message Actions QA A",
        "mcp message actions login A",
        "mcp message actions gate A",
        QaParticipantLoginGate::BootstrapNewIdentity,
    )
    .await?;
    let QaParticipantLoginOutcome {
        runtime: runtime_b,
        conn: mut conn_b,
        account_key: account_key_b,
        ..
    } = login_synced_participant_for_qa(
        &config.homeserver,
        qa_data_dir("mcp-message-actions-b"),
        &config.user_b,
        &config.password_b,
        "Koushi MCP Message Actions QA B",
        "mcp message actions login B",
        "mcp message actions gate B",
        QaParticipantLoginGate::BootstrapNewIdentity,
    )
    .await?;
    let sender_a_id = format!("@{}:{}", config.user_a, config.server_name);
    let user_b_id = format!("@{}:{}", config.user_b, config.server_name);

    let flow = run_mcp_message_actions_flow(
        &mut conn_a,
        &account_key_a,
        &sender_a_id,
        &mut conn_b,
        &user_b_id,
    )
    .await;
    let cleanup_b = cleanup_logged_in_runtime(
        conn_b,
        runtime_b,
        account_key_b,
        "mcp message actions cleanup B",
    )
    .await;
    let cleanup_a = cleanup_logged_in_runtime(
        conn_a,
        runtime_a,
        account_key_a,
        "mcp message actions cleanup A",
    )
    .await;
    flow?;
    cleanup_b.and(cleanup_a)?;
    println!("mcp_message_actions=ok");
    Ok(())
}

async fn run_mcp_message_actions_flow(
    conn_a: &mut CoreConnection,
    account_key_a: &AccountKey,
    sender_a_id: &str,
    conn_b: &mut CoreConnection,
    user_b_id: &str,
) -> Result<(), String> {
    let room_id = create_room_for_qa(
        conn_a,
        "Synthetic MCP Message Actions Room",
        false,
        "mcp message actions room",
    )
    .await?;
    let room_key_a = TimelineKey {
        account_key: account_key_a.clone(),
        kind: TimelineKind::Room {
            room_id: room_id.clone(),
        },
    };
    subscribe_timeline_for_qa(conn_a, &room_key_a, "mcp message actions subscribe A room").await?;

    let body_event_id = send_text(
        conn_a,
        &room_key_a,
        "mcp-message-actions-body",
        BODY,
        "mcp message actions body send",
    )
    .await?;
    let deleted_event_id = send_text(
        conn_a,
        &room_key_a,
        "mcp-message-actions-deleted",
        DELETED_BODY,
        "mcp message actions deleted send",
    )
    .await?;
    let root_event_id = send_text(
        conn_a,
        &room_key_a,
        "mcp-message-actions-root",
        THREAD_ROOT_BODY,
        "mcp message actions root send",
    )
    .await?;

    // The observer is a full member of the room, so B reads the same events the
    // homeserver holds rather than anything A projected locally.
    invite_user_for_qa(conn_a, &room_id, user_b_id, "mcp message actions invite B").await?;
    wait_for_invite_in_snapshot(conn_b, &room_id, None, "mcp message actions invite B").await?;
    accept_invite_for_qa(conn_b, &room_id, "mcp message actions B joins").await?;
    wait_for_room_in_room_list(conn_b, &room_id, "mcp message actions B room list").await?;

    let view = read_message(conn_a, &room_id, &body_event_id, "mcp message actions read").await?;
    check(
        view.sender_id == sender_a_id,
        "mcp message actions read: sender is not the account that posted the message",
    )?;
    check(
        view.timestamp_ms > 0,
        "mcp message actions read: the read carried no timestamp",
    )?;
    check(
        view.kind == RoomMessageKind::Text,
        &format!(
            "mcp message actions read: kind was {:?}, expected a text message",
            view.kind
        ),
    )?;
    check(
        view.content_available,
        "mcp message actions read: a readable message reported content_available=false",
    )?;
    check(
        view.body.as_deref() == Some(BODY),
        "mcp message actions read: the read body was not the posted text",
    )?;
    check(
        !view.is_redacted,
        "mcp message actions read: a freshly posted message read as redacted",
    )?;

    let added = react(
        conn_a,
        &room_id,
        &root_event_id,
        REACTION_KEY,
        "mcp message actions react",
    )
    .await?;
    check(
        added.added,
        "mcp message actions react: a first reaction reported added=false",
    )?;
    let reaction_event_id = added.reaction_event_id.ok_or_else(|| {
        "mcp message actions react: the reaction carried no reaction event id".to_owned()
    })?;
    let duplicate = react(
        conn_a,
        &room_id,
        &root_event_id,
        REACTION_KEY,
        "mcp message actions react again",
    )
    .await?;
    check(
        !duplicate.added,
        "mcp message actions react again: the same key reported added=true, so a duplicate was sent",
    )?;
    check(
        duplicate.reaction_event_id.as_deref() == Some(reaction_event_id.as_str()),
        "mcp message actions react again: the existing reaction event id was not reported",
    )?;
    let removed = unreact(
        conn_a,
        &room_id,
        &root_event_id,
        REACTION_KEY,
        "mcp message actions unreact",
    )
    .await?;
    check(
        removed == 1,
        &format!("mcp message actions unreact: removed {removed} reactions, expected 1"),
    )?;
    let removed_again = unreact(
        conn_a,
        &room_id,
        &root_event_id,
        REACTION_KEY,
        "mcp message actions unreact again",
    )
    .await?;
    check(
        removed_again == 0,
        &format!(
            "mcp message actions unreact again: removed {removed_again} reactions, expected 0"
        ),
    )?;
    // The observer read below compares a live reaction, so the removed reaction
    // goes back through the same set semantics that were just proven.
    let restored = react(
        conn_a,
        &room_id,
        &root_event_id,
        REACTION_KEY,
        "mcp message actions restore reaction",
    )
    .await?;
    check(
        restored.added,
        "mcp message actions restore reaction: re-adding the reaction reported added=false",
    )?;

    let formatting = conn_a
        .snapshot()
        .settings
        .values
        .composer
        .formatting_options();
    let edit = edit_message(
        conn_a,
        &room_id,
        &body_event_id,
        EDITED_BODY,
        formatting,
        "mcp message actions edit",
    )
    .await?;
    check(
        edit.edit_event_id.is_some(),
        "mcp message actions edit: the edit carried no edit event id",
    )?;
    // A retry with the same client_request_id is answered from Core's ledger
    // with the first result; a second edit would carry a new event id and
    // become the edit the next read reports (#1681).
    let edit_retry = edit_message(
        conn_a,
        &room_id,
        &body_event_id,
        EDITED_BODY,
        formatting,
        "mcp message actions edit retry",
    )
    .await?;
    check(
        edit_retry.edit_event_id == edit.edit_event_id,
        "mcp message actions edit retry: the retry did not replay the first edit event id",
    )?;
    let edited = read_message(
        conn_a,
        &room_id,
        &body_event_id,
        "mcp message actions read edited",
    )
    .await?;
    check(
        edited.body.as_deref() == Some(EDITED_BODY),
        "mcp message actions read edited: the read did not return the edited body",
    )?;
    check(
        edited.edited.as_ref().map(|edit| edit.event_id.as_str()) == edit.edit_event_id.as_deref(),
        "mcp message actions read edited: the applied edit is not the one edit sent",
    )?;

    let redacted = delete_message(
        conn_a,
        &room_id,
        &deleted_event_id,
        "mcp message actions delete",
    )
    .await?;
    check(
        !redacted.already_redacted,
        "mcp message actions delete: the first delete reported already_redacted=true",
    )?;
    check(
        redacted.redaction_event_id.is_some(),
        "mcp message actions delete: the redaction carried no redaction event id",
    )?;
    let redacted_again = delete_message(
        conn_a,
        &room_id,
        &deleted_event_id,
        "mcp message actions delete again",
    )
    .await?;
    check(
        redacted_again.already_redacted,
        "mcp message actions delete again: the second delete reported already_redacted=false",
    )?;
    check(
        redacted_again.redaction_event_id.is_none(),
        "mcp message actions delete again: the second delete reported a new redaction event id",
    )?;
    let deleted = read_message(
        conn_a,
        &room_id,
        &deleted_event_id,
        "mcp message actions read deleted",
    )
    .await?;
    check(
        deleted.is_redacted,
        "mcp message actions read deleted: the redacted message did not read as redacted",
    )?;
    check(
        deleted.body.is_none() && !deleted.content_available,
        "mcp message actions read deleted: the redacted message still carried a body",
    )?;

    let reply = thread_reply(
        conn_a,
        &room_id,
        &root_event_id,
        THREAD_REPLY_BODY,
        formatting,
        "mcp message actions thread reply",
    )
    .await?;
    check(
        !reply.event_id.is_empty(),
        "mcp message actions thread reply: the reply carried no event id",
    )?;
    let reply_retry = thread_reply(
        conn_a,
        &room_id,
        &root_event_id,
        THREAD_REPLY_BODY,
        formatting,
        "mcp message actions thread reply retry",
    )
    .await?;
    check(
        reply_retry.event_id == reply.event_id
            && reply_retry.transaction_id == reply.transaction_id,
        "mcp message actions thread reply retry: the retry posted instead of replaying the first reply",
    )?;
    let reply_view = read_message(
        conn_a,
        &room_id,
        &reply.event_id,
        "mcp message actions read reply",
    )
    .await?;
    check(
        reply_view.body.as_deref() == Some(THREAD_REPLY_BODY),
        "mcp message actions read reply: the read did not return the reply text",
    )?;
    check(
        reply_view.thread_root_event_id.as_deref() == Some(root_event_id.as_str()),
        "mcp message actions read reply: the read did not report the thread root",
    )?;

    // Observer phase: another account runtime re-reads the same events, so every
    // effect below has to be on the homeserver, not in A's store.
    let observed_edit = read_message(
        conn_b,
        &room_id,
        &body_event_id,
        "mcp message actions observer read edited",
    )
    .await?;
    check(
        observed_edit.body.as_deref() == Some(EDITED_BODY),
        "mcp message actions observer read edited: B did not see the edited body",
    )?;
    let observed_deleted = read_message(
        conn_b,
        &room_id,
        &deleted_event_id,
        "mcp message actions observer read deleted",
    )
    .await?;
    check(
        observed_deleted.is_redacted && observed_deleted.body.is_none(),
        "mcp message actions observer read deleted: B did not see the redaction",
    )?;
    let observed_root = read_message(
        conn_b,
        &room_id,
        &root_event_id,
        "mcp message actions observer read reaction",
    )
    .await?;
    let observed_reaction = observed_root
        .reactions
        .iter()
        .find(|reaction| reaction.key == REACTION_KEY);
    check(
        observed_reaction.is_some_and(|reaction| reaction.count == 1 && !reaction.mine),
        "mcp message actions observer read reaction: B did not see one foreign reaction",
    )?;
    let observed_reply = read_message(
        conn_b,
        &room_id,
        &reply.event_id,
        "mcp message actions observer read reply",
    )
    .await?;
    check(
        observed_reply.thread_root_event_id.as_deref() == Some(root_event_id.as_str()),
        "mcp message actions observer read reply: B did not see the thread relation",
    )?;
    Ok(())
}

fn check(condition: bool, detail: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(detail.to_owned())
    }
}

async fn send_text(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    transaction_id: &str,
    body: &str,
    label: &str,
) -> Result<String, String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::SendText {
        request_id,
        key: key.clone(),
        transaction_id: transaction_id.to_owned(),
        document: koushi_state::ComposerDocument::from_plain_text(body.to_owned()),
    }))
    .await
    .map_err(|e| format!("{label}: send submission failed: {e}"))?;
    Ok(
        wait_for_send_flow_completion(conn, request_id, key, transaction_id, body, label)
            .await?
            .event_id,
    )
}

/// Runs one direct message action and hands its response to `extract`.
///
/// The request's `Debug` is its case only, so a failure line never carries a
/// room id, an event id, a body, or a reaction key.
async fn direct<Response>(
    conn: &CoreConnection,
    request: DirectMessageRequest,
    label: &str,
    extract: impl FnOnce(DirectMessageResponse) -> Option<Response>,
) -> Result<Response, String> {
    let case = format!("{request:?}");
    let response = conn
        .direct_message_action(request)
        .await
        .map_err(|failure| format!("{label}: {case} failed with {failure:?}"))?;
    extract(response).ok_or_else(|| format!("{label}: {case} answered a different action"))
}

async fn read_message(
    conn: &CoreConnection,
    room_id: &str,
    event_id: &str,
    label: &str,
) -> Result<DirectMessageView, String> {
    direct(
        conn,
        DirectMessageRequest::Read {
            room_id: room_id.to_owned(),
            event_id: event_id.to_owned(),
        },
        label,
        |response| match response {
            DirectMessageResponse::Read(view) => Some(view),
            _ => None,
        },
    )
    .await
}

async fn react(
    conn: &CoreConnection,
    room_id: &str,
    event_id: &str,
    key: &str,
    label: &str,
) -> Result<DirectReactionOutcome, String> {
    direct(
        conn,
        DirectMessageRequest::React {
            room_id: room_id.to_owned(),
            event_id: event_id.to_owned(),
            key: key.to_owned(),
        },
        label,
        |response| match response {
            DirectMessageResponse::React(outcome) => Some(outcome),
            _ => None,
        },
    )
    .await
}

async fn unreact(
    conn: &CoreConnection,
    room_id: &str,
    event_id: &str,
    key: &str,
    label: &str,
) -> Result<u32, String> {
    direct(
        conn,
        DirectMessageRequest::Unreact {
            room_id: room_id.to_owned(),
            event_id: event_id.to_owned(),
            key: key.to_owned(),
        },
        label,
        |response| match response {
            DirectMessageResponse::Unreact(removed) => Some(removed),
            _ => None,
        },
    )
    .await
}

/// One fixed retry key per action, as an MCP caller retrying the same call
/// would send it.
fn qa_retry_key(client_request_id: &str) -> koushi_core::DirectRetryKey {
    koushi_core::DirectRetryKey {
        client: "qa".to_owned(),
        client_request_id: client_request_id.to_owned(),
    }
}

async fn edit_message(
    conn: &CoreConnection,
    room_id: &str,
    event_id: &str,
    body: &str,
    formatting: koushi_state::ComposerFormattingOptions,
    label: &str,
) -> Result<DirectEditOutcome, String> {
    direct(
        conn,
        DirectMessageRequest::Edit {
            room_id: room_id.to_owned(),
            event_id: event_id.to_owned(),
            body: body.to_owned(),
            formatting,
            retry: Some(qa_retry_key("edit")),
        },
        label,
        |response| match response {
            DirectMessageResponse::Edit(outcome) => Some(outcome),
            _ => None,
        },
    )
    .await
}

async fn delete_message(
    conn: &CoreConnection,
    room_id: &str,
    event_id: &str,
    label: &str,
) -> Result<DirectRedactOutcome, String> {
    direct(
        conn,
        DirectMessageRequest::Delete {
            room_id: room_id.to_owned(),
            event_id: event_id.to_owned(),
        },
        label,
        |response| match response {
            DirectMessageResponse::Delete(outcome) => Some(outcome),
            _ => None,
        },
    )
    .await
}

async fn thread_reply(
    conn: &CoreConnection,
    room_id: &str,
    root_event_id: &str,
    body: &str,
    formatting: koushi_state::ComposerFormattingOptions,
    label: &str,
) -> Result<DirectThreadReplyOutcome, String> {
    direct(
        conn,
        DirectMessageRequest::ThreadReply {
            room_id: room_id.to_owned(),
            root_event_id: root_event_id.to_owned(),
            body: body.to_owned(),
            formatting,
            retry: Some(qa_retry_key("thread-reply")),
        },
        label,
        |response| match response {
            DirectMessageResponse::ThreadReply(outcome) => Some(outcome),
            _ => None,
        },
    )
    .await
}
