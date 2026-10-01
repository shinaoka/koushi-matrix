//! Two-user Verify user QA (#1024) against a disposable local homeserver.
//!
//! User A (Koushi Core) opens B's User info and requests verification in the
//! direct chat with B; B (a second Koushi Core) accepts the in-room request,
//! both sides see the same seven SAS emoji and confirm, and A's contact
//! security details then report B as verified by A while B's device
//! confirmation stays independent.
//!
//! Tuwunel and Synapse both exercise a fresh encrypted DM. The QA waits for
//! the outgoing verification-progress event, emitted only after the SDK send
//! completes, before B joins. This makes Synapse's pre-join event-delivery
//! behavior deterministic instead of racing the command's optimistic state.

use koushi_protocol::command::ContactSecurityRequest;
use koushi_state::{
    AppState, ContactDevicesStatus, ContactIdentityVerification, ContactSecurityLoadState,
    ContactVerificationDirectChat, ContactVerificationOffer, VerificationInitiator,
};

use super::event_wait::{
    QaEventDeadline, wait_for_dm_room_in_room_list, wait_for_invite_in_snapshot,
};
use super::fixtures::{accept_invite_for_qa, start_direct_message_for_qa};
use super::participants::{
    authenticated_session_info, verification_state_sas, wait_for_verification_accepted,
    wait_for_verification_requested_event_only,
};
use super::registry::{E2EE_EVENT_TIMEOUT, QaConfig};
use super::{
    AccountCommand, CoreCommand, CoreConnection, CoreEvent, E2eeTrustEvent, SasEmoji,
    VerificationFlowState, VerificationTarget,
};

async fn wait_until<T>(
    conn: &mut CoreConnection,
    label: &str,
    mut check: impl FnMut(&AppState) -> Result<Option<T>, String>,
) -> Result<T, String> {
    let deadline = QaEventDeadline::after(E2EE_EVENT_TIMEOUT);
    loop {
        if let Some(value) = check(&conn.snapshot())? {
            return Ok(value);
        }
        deadline
            .recv(conn)
            .await
            .map_err(|_| format!("{label}: timed out"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;
    }
}

async fn contact_security(
    conn: &mut CoreConnection,
    request: ContactSecurityRequest,
    label: &str,
) -> Result<(), String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Account(AccountCommand::ContactSecurity {
        request_id,
        request,
    }))
    .await
    .map_err(|error| format!("{label}: {error}"))
}

async fn wait_for_sas(
    conn: &mut CoreConnection,
    flow_id: u64,
    label: &str,
) -> Result<Vec<SasEmoji>, String> {
    wait_until(conn, label, |state| {
        verification_state_sas(&state.e2ee_trust.verification, flow_id, label)
    })
    .await
}

async fn wait_for_done(conn: &mut CoreConnection, flow_id: u64, label: &str) -> Result<(), String> {
    wait_until(conn, label, |state| match &state.e2ee_trust.verification {
        VerificationFlowState::Done { request_id, .. } if *request_id == flow_id => Ok(Some(())),
        VerificationFlowState::Failed {
            request_id, kind, ..
        } if *request_id == flow_id => Err(format!("{label}: verification failed: {kind:?}")),
        _ => Ok(None),
    })
    .await
}

async fn wait_for_outgoing_request_sent(
    conn: &mut CoreConnection,
    flow_id: u64,
    user_id: &str,
    label: &str,
) -> Result<(), String> {
    let deadline = QaEventDeadline::after(E2EE_EVENT_TIMEOUT);
    loop {
        if let VerificationFlowState::Failed {
            request_id, kind, ..
        } = &conn.snapshot().e2ee_trust.verification
            && *request_id == flow_id
        {
            return Err(format!("{label}: verification request failed: {kind:?}"));
        }

        let event = deadline
            .recv(conn)
            .await
            .map_err(|_| format!("{label}: timed out waiting for the SDK request send"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;
        if let CoreEvent::E2eeTrust(E2eeTrustEvent::VerificationProgress { state, .. }) = event {
            match state {
                VerificationFlowState::Requested {
                    request_id,
                    target,
                    initiator: VerificationInitiator::Us,
                } if request_id == flow_id && target.user_id == user_id => return Ok(()),
                VerificationFlowState::Failed {
                    request_id, kind, ..
                } if request_id == flow_id => {
                    return Err(format!("{label}: verification request failed: {kind:?}"));
                }
                _ => {}
            }
        }
    }
}

pub(super) async fn run_user_verification_stage(
    config: &QaConfig,
    conn_a: &mut CoreConnection,
    conn_b: &mut CoreConnection,
) -> Result<(), String> {
    let session_a = authenticated_session_info(conn_a, "user_verification session A")?;
    let session_b = authenticated_session_info(conn_b, "user_verification session B")?;
    let user_b = session_b.user_id.clone();
    let new_direct_chat = matches!(config.server_kind.as_str(), "tuwunel" | "synapse");

    // Existing-chat path: A and B already share a direct chat.
    if !new_direct_chat {
        let room_id =
            start_direct_message_for_qa(conn_a, &user_b, "user_verification A starts DM").await?;
        wait_for_dm_room_in_room_list(conn_a, &room_id, "user_verification A DM listed").await?;
        wait_for_invite_in_snapshot(conn_b, &room_id, Some(true), "user_verification DM invite")
            .await?;
        accept_invite_for_qa(conn_b, &room_id, "user_verification B joins DM").await?;
    }
    let expected_chat = |chat: ContactVerificationDirectChat| {
        (chat == ContactVerificationDirectChat::New) == new_direct_chat
    };

    // 1. A opens B's User info: owner-confirmed devices, not verified by A,
    // Verify user offered in the direct chat it will use.
    contact_security(
        conn_a,
        ContactSecurityRequest::Load {
            user_id: user_b.clone(),
        },
        "user_verification load B",
    )
    .await?;
    wait_until(conn_a, "user_verification offered", |state| {
        let contact = &state.contact_security;
        if contact.user_id.as_deref() != Some(user_b.as_str()) {
            return Ok(None);
        }
        match (&contact.load, &contact.summary) {
            (ContactSecurityLoadState::Failed { failure_kind, .. }, _) => Err(format!(
                "user_verification: retrieval failed: {failure_kind:?}"
            )),
            (ContactSecurityLoadState::Loaded { .. }, Some(summary)) => {
                if summary.identity != ContactIdentityVerification::NotVerifiedByYou
                    || summary.devices != ContactDevicesStatus::AllOwnerSigned
                    || !matches!(
                        summary.verification,
                        ContactVerificationOffer::Offered { direct_chat } if expected_chat(direct_chat)
                    )
                {
                    return Err(format!(
                        "user_verification: unexpected initial summary {summary:?}"
                    ));
                }
                Ok(Some(()))
            }
            _ => Ok(None),
        }
    })
    .await?;
    println!("user_verification_offered=ok");

    // 2. A sends the request. It waits for B instead of offering Accept.
    let request_id = conn_a.next_request_id();
    let flow_a = request_id.sequence;
    conn_a
        .command(CoreCommand::Account(AccountCommand::ContactSecurity {
            request_id,
            request: ContactSecurityRequest::RequestVerification {
                user_id: user_b.clone(),
            },
        }))
        .await
        .map_err(|error| format!("user_verification request: {error}"))?;
    wait_for_outgoing_request_sent(conn_a, flow_a, &user_b, "user_verification request sent")
        .await?;
    println!("user_verification_request_sent=ok");

    // 3. B accepts the in-room request (joining the new direct chat first).
    if new_direct_chat {
        let dm_room_id = wait_until(conn_b, "user_verification DM invite", |state| {
            // B is a fresh QA account, so its only invite is the room A just
            // created. Do not depend on the server preserving is_direct in
            // the invited-room projection.
            Ok(state.invites.first().map(|invite| invite.room_id.clone()))
        })
        .await?;
        accept_invite_for_qa(conn_b, &dm_room_id, "user_verification B joins DM").await?;
        println!("user_verification_dm_joined=ok");
    }
    let target_a = VerificationTarget {
        user_id: session_a.user_id.clone(),
        device_id: session_a.device_id.clone(),
    };
    let flow_b = wait_for_verification_requested_event_only(
        conn_b,
        Some(&target_a),
        None,
        "user_verification B incoming request",
    )
    .await?;
    println!("user_verification_incoming_request=ok");
    // The reducer projects an incoming request as acceptable (initiator them).
    wait_until(
        conn_b,
        "user_verification B acceptable request",
        |state| match &state.e2ee_trust.verification {
            VerificationFlowState::Requested {
                request_id,
                initiator: VerificationInitiator::Them,
                ..
            } if *request_id == flow_b => Ok(Some(())),
            VerificationFlowState::Requested {
                request_id,
                initiator: VerificationInitiator::Us,
                ..
            } if *request_id == flow_b => {
                Err("user_verification: B's incoming request is not acceptable".to_owned())
            }
            VerificationFlowState::Failed {
                request_id, kind, ..
            } if *request_id == flow_b => {
                Err(format!("user_verification: B's request failed: {kind:?}"))
            }
            _ => Ok(None),
        },
    )
    .await?;
    let accept_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Account(AccountCommand::AcceptVerification {
            request_id: accept_id,
            flow_id: flow_b,
        }))
        .await
        .map_err(|error| format!("user_verification accept: {error}"))?;
    wait_for_verification_accepted(conn_b, flow_b, Some(accept_id), "user_verification B ready")
        .await?;
    println!("user_verification_accepted=ok");

    // 4. A (the requester) starts SAS; both see the same seven emoji.
    let emojis_a = wait_for_sas(conn_a, flow_a, "user_verification A SAS").await?;
    let emojis_b = wait_for_sas(conn_b, flow_b, "user_verification B SAS").await?;
    if emojis_a.len() != 7 || emojis_a != emojis_b {
        return Err("user_verification: SAS emoji differ".to_owned());
    }
    println!("user_verification_sas_match=ok");

    for (conn, flow_id, label) in [
        (&mut *conn_a, flow_a, "user_verification confirm A"),
        (&mut *conn_b, flow_b, "user_verification confirm B"),
    ] {
        let request_id = conn.next_request_id();
        conn.command(CoreCommand::Account(
            AccountCommand::ConfirmSasVerification {
                request_id,
                flow_id,
            },
        ))
        .await
        .map_err(|error| format!("{label}: {error}"))?;
    }
    wait_for_done(conn_a, flow_a, "user_verification A done").await?;
    wait_for_done(conn_b, flow_b, "user_verification B done").await?;
    println!("user_verification_done=ok");

    // 5. A's open User info refreshes: B is verified by A, no longer offered,
    // and B's device confirmation is unchanged by A's verification.
    wait_until(conn_a, "user_verification identity verified", |state| {
        let Some(summary) = state
            .contact_security
            .summary
            .as_ref()
            .filter(|_| state.contact_security.user_id.as_deref() == Some(user_b.as_str()))
        else {
            return Ok(None);
        };
        if summary.identity != ContactIdentityVerification::VerifiedByYou {
            return Ok(None);
        }
        if summary.verification != ContactVerificationOffer::NotOffered
            || summary.devices != ContactDevicesStatus::AllOwnerSigned
        {
            return Err(format!(
                "user_verification: unexpected verified summary {summary:?}"
            ));
        }
        Ok(Some(()))
    })
    .await?;
    println!("user_verification_identity_verified=ok");

    contact_security(
        conn_a,
        ContactSecurityRequest::Close,
        "user_verification close",
    )
    .await?;
    println!("user_verification=ok");
    Ok(())
}
