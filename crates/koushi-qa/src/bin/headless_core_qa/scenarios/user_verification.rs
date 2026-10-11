//! Two-user Verify user QA (#1024) against a disposable local homeserver.
//!
//! User A (Koushi Core) opens B's User info and requests verification in the
//! direct chat with B; B (a second Koushi Core) accepts the in-room request,
//! both sides see the same seven SAS emoji and confirm, and A's contact
//! security details then report B as verified by A while B's device
//! confirmation stays independent.
//!
//! Tuwunel and Synapse both exercise a fresh encrypted DM. A starts the request
//! while B is still invited; the QA joins B before waiting for the SDK send to
//! finish. This verifies the sender waits for membership before emitting the
//! room-based verification request.

use std::time::Duration;

use koushi_protocol::command::ContactSecurityRequest;
use koushi_state::{
    AppState, ContactDevicesStatus, ContactIdentityVerification, ContactSecurityLoadState,
    ContactVerificationDirectChat, ContactVerificationOffer, VerificationInitiator,
};

use super::event_wait::{
    QaEventDeadline, subscribe_timeline_for_qa, timeline_item_is_decryption_failure,
    wait_for_dm_room_in_room_list, wait_for_invite_in_snapshot,
};
use super::fixtures::{
    accept_invite_for_qa, load_room_settings_for_qa, start_direct_message_for_qa,
};
use super::participants::{
    authenticated_session_info, incoming_request_timeout_context, verification_state_sas,
    wait_for_verification_accepted, wait_for_verification_requested_event_only,
};
use super::registry::{E2EE_EVENT_TIMEOUT, QaConfig};
use super::{
    AccountCommand, AccountKey, CoreCommand, CoreConnection, CoreEvent, E2eeTrustEvent, SasEmoji,
    SessionState, TimelineItem, TimelineKey, VerificationFlowState, VerificationTarget,
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

/// Members A's own room-list projection reports for the direct chat with B.
fn own_dm_joined_members(state: &AppState, user_b: &str) -> u64 {
    state
        .rooms
        .iter()
        .find(|room| room.is_dm && room.dm_user_ids.iter().any(|id| id == user_b))
        .map(|room| room.joined_members)
        .unwrap_or(0)
}

/// Joined members A's room member projection reports, as a private-data-free
/// count, through the room's own settings projection.
async fn own_dm_settings_joined_members(
    conn: &mut CoreConnection,
    user_b: &str,
    label: &str,
) -> usize {
    let Some(room_id) = conn
        .snapshot()
        .rooms
        .iter()
        .find(|room| room.is_dm && room.dm_user_ids.iter().any(|id| id == user_b))
        .map(|room| room.room_id.clone())
    else {
        return 0;
    };
    match load_room_settings_for_qa(conn, &room_id, label).await {
        Ok(settings) => settings
            .members
            .iter()
            .filter(|member| {
                matches!(
                    member.membership,
                    koushi_state::RoomMemberMembership::Joined
                )
            })
            .count(),
        Err(_) => 0,
    }
}

/// Private-data-free evidence for A's in-room verification send.
#[derive(Debug, Clone, Copy)]
struct OutgoingRequestSend {
    /// A's own room-list projection of the direct chat with B.
    joined_members: u64,
    /// The account actor published the post-send progress event for this flow.
    event_seen: bool,
}

impl OutgoingRequestSend {
    /// `finished` only when the post-send event was observed; `optimistic_only`
    /// means A's state shows `Requested` purely from the command-acceptance
    /// projection.
    fn token(self) -> &'static str {
        if self.event_seen {
            "finished"
        } else {
            "optimistic_only"
        }
    }
}

/// Wait until A's in-room verification send is established, and return the
/// evidence for it.
///
/// `ContactSecurityRequest::RequestVerification` reaches
/// `VerificationFlowState::Requested { initiator: Us }` in A's state the moment
/// the command is accepted (`account_command_projected_action`), while Koushi's
/// vendored SDK patch (`wait_for_room_member_to_join`, 60s `JOIN_TIMEOUT`) has
/// not sent the in-room request yet and can still fail with
/// `crate::Error::Timeout`. The projected state is therefore identical before
/// and after the send and cannot stand in for it; the account actor publishes
/// `VerificationProgress { state: Requested { initiator: Us } }` only from the
/// `Ok` branch of `koushi_sdk::request_user_verification` (and
/// `VerificationFailed` when it did not succeed), so that event is the
/// authoritative post-send signal.
///
/// Both halves of the precondition are tracked in this single event loop: a
/// snapshot-only check would accept the command-acceptance projection as proof
/// of the send (#1169), and splitting the join wait into a loop of its own could
/// swallow the post-send event before it is observed.
///
/// The observation is a room-list snapshot read, never a timeline open, so a
/// run that reaches its checkpoints keeps the direct chat unloaded. B's timeline
/// is opened only on the failure path, by `probe_b_dm_timeline`.
async fn wait_for_outgoing_request_sent(
    conn: &mut CoreConnection,
    flow_id: u64,
    user_id: &str,
    label: &str,
) -> Result<OutgoingRequestSend, String> {
    let deadline = QaEventDeadline::after(E2EE_EVENT_TIMEOUT);
    let mut observation = OutgoingRequestSend {
        joined_members: 0,
        event_seen: false,
    };
    loop {
        observation.joined_members = observation
            .joined_members
            .max(own_dm_joined_members(&conn.snapshot(), user_id));
        if let VerificationFlowState::Failed {
            request_id, kind, ..
        } = &conn.snapshot().e2ee_trust.verification
            && *request_id == flow_id
        {
            return Err(format!("{label}: verification request failed: {kind:?}"));
        }
        if observation.event_seen && observation.joined_members >= 2 {
            return Ok(observation);
        }
        if tokio::time::Instant::now() >= deadline.instant {
            // Cross-check the room member projection so the token says whether
            // A never observed the join or this server does not report the
            // room-list count at all.
            let member_projection = own_dm_settings_joined_members(conn, user_id, label).await;
            return Err(format!(
                "{label}: timed out waiting for the SDK request send a_send={} \
                 joined_members={} member_projection_joined={member_projection}",
                observation.token(),
                observation.joined_members,
            ));
        }

        let event = deadline
            .recv(conn)
            .await
            .map_err(|_| {
                format!(
                    "{label}: timed out waiting for the SDK request send a_send={} \
                     joined_members={}",
                    observation.token(),
                    observation.joined_members,
                )
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;
        if let CoreEvent::E2eeTrust(E2eeTrustEvent::VerificationProgress { state, .. }) = event {
            match state {
                VerificationFlowState::Requested {
                    request_id,
                    target,
                    initiator: VerificationInitiator::Us,
                } if request_id == flow_id && target.user_id == user_id => {
                    observation.event_seen = true;
                }
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

/// Bound for the failure-path timeline probe. It exists so the probe can never
/// hold the scenario open; a probe that does not answer in time reports
/// `unknown` and the original failure stands.
const B_DM_TIMELINE_PROBE_TIMEOUT: Duration = Duration::from_secs(15);

/// Whether B's direct-chat timeline holds an item it cannot decrypt.
///
/// A's in-room verification request is the only encrypted message this direct
/// chat ever carries, so an item B reports as a decryption failure is that
/// request: received, and not decodable from the first sync that delivered it.
/// The negative answer is deliberately not named "never received": a request B
/// re-decrypted once the room key arrived, or one the timeline never listed,
/// looks the same here.
fn b_dm_timeline_request_state(items: &[TimelineItem]) -> &'static str {
    if items.iter().any(timeline_item_is_decryption_failure) {
        "undecryptable"
    } else {
        "no_undecryptable_item"
    }
}

/// Failure-path diagnosis only: read B's direct-chat timeline, once.
///
/// The passing path of this scenario never opens a timeline, so a run that
/// reaches its checkpoints leaves the direct chat unloaded; this probe runs only
/// after a B-side timeout has already been decided, and its answer cannot turn
/// that failure into a pass. Conditions: private-data-free (a coarse state
/// only), bounded by [`B_DM_TIMELINE_PROBE_TIMEOUT`], and `unknown` on any
/// subscription error.
async fn probe_b_dm_timeline(conn_b: &mut CoreConnection, room_id: &str) -> &'static str {
    let account_key = match &conn_b.snapshot().session {
        SessionState::Ready(info) => AccountKey(info.user_id.clone()),
        _ => return "unknown",
    };
    let key = TimelineKey::room(account_key, room_id.to_owned());
    let probe = async {
        let items =
            subscribe_timeline_for_qa(conn_b, &key, "user_verification B timeline probe").await?;
        Ok::<_, String>(b_dm_timeline_request_state(&items))
    };
    match tokio::time::timeout(B_DM_TIMELINE_PROBE_TIMEOUT, probe).await {
        Ok(Ok(state)) => state,
        Ok(Err(_)) | Err(_) => "unknown",
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
    // 3. B accepts the invite before A's SDK send completes. The SDK must
    // hold the room-based request until B's membership is joined.
    //
    // The room id is kept as evidence rather than as a gate. Whether B's own
    // room-list projection has processed the joined room at the moment A's send
    // lands decides which delivery interleaving B is in: when it has not, the
    // room and the encrypted request reach B together in one room-list
    // response, ahead of the megolm key on the separate encryption connection.
    // Waiting for B's room list before A may send would remove exactly that
    // interleaving, which is the one the nightly lane fails on, so the QA
    // records it instead of forbidding it.
    let mut dm_room_id = None;
    if new_direct_chat {
        let room_id = wait_until(conn_b, "user_verification DM invite", |state| {
            // B is a fresh QA account, so its only invite is the room A just
            // created. Do not depend on the server preserving is_direct in
            // the invited-room projection.
            Ok(state.invites.first().map(|invite| invite.room_id.clone()))
        })
        .await?;
        accept_invite_for_qa(conn_b, &room_id, "user_verification B joins DM").await?;
        println!("user_verification_dm_joined=ok");
        dm_room_id = Some(room_id);
    }
    // A's command was accepted, but the SDK sends the in-room request only once
    // A's own projection contains B as a joined member, and only the actor's
    // post-send progress event proves that send happened (#1169).
    let send =
        wait_for_outgoing_request_sent(conn_a, flow_a, &user_b, "user_verification request sent")
            .await?;
    println!(
        "user_verification_a_observed_join=ok joined_members={}",
        send.joined_members
    );
    println!("user_verification_request_sent=ok a_send={}", send.token());

    // Sampled once A's send is established and before B is asked for anything,
    // so both green and failing runs say which delivery interleaving B was in.
    let b_dm_joined_members_at_send = dm_room_id
        .as_deref()
        .and_then(|room_id| {
            conn_b
                .snapshot()
                .rooms
                .iter()
                .find(|room| room.room_id == room_id)
                .map(|room| room.joined_members)
        })
        .unwrap_or(0);
    println!(
        "user_verification_b_dm_room_listed_at_send={}",
        dm_room_id.is_some() && b_dm_joined_members_at_send > 0
    );

    let target_a = VerificationTarget {
        user_id: session_a.user_id.clone(),
        device_id: session_a.device_id.clone(),
    };
    let flow_b = match wait_for_verification_requested_event_only(
        conn_b,
        Some(&target_a),
        None,
        "user_verification B incoming request",
    )
    .await
    {
        Ok(flow_b) => flow_b,
        Err(error) => {
            // Say which half was missing: B's own projection first (#1279), then
            // what A's flow reached instead.
            // `a_send=finished` was established before this wait started, so a
            // B-side timeout names B's delivery/decryption of the in-room
            // request, never A's send. `b_dm_room_listed_at_send=false` says B's
            // room list learned about the DM from the same response that carried
            // the request, so the room key could not have been known yet.
            let b_snapshot = conn_b.snapshot();
            let context = incoming_request_timeout_context(
                &b_snapshot.e2ee_trust.verification,
                Some(&target_a),
                &b_snapshot.sync,
            );
            // Failure-path-only diagnosis: the verdict above already stands.
            let b_dm_timeline = match dm_room_id.as_deref() {
                Some(room_id) => probe_b_dm_timeline(conn_b, room_id).await,
                None => "not_attempted",
            };
            return Err(format!(
                "{error} a_send={} a_room_list_joined_members={} \
                 b_dm_room_listed_at_send={} b_dm_joined_members_at_send={} \
                 b_dm_timeline={b_dm_timeline} {context}",
                send.token(),
                send.joined_members,
                dm_room_id.is_some() && b_dm_joined_members_at_send > 0,
                b_dm_joined_members_at_send,
            ));
        }
    };
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
