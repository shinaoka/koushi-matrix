use super::diagnostics::{
    gate_session_phase, session_gate_closed_summary, verification_closed_summary,
    verification_event_stream_error, verification_state_flow_id,
};
use super::event_wait::{
    PairedEventWaitError, QaEventDeadline, start_sync_for_qa, wait_for_logged_in,
    wait_for_logged_out, wait_for_paired_event_until, wait_for_ready_snapshot,
    wait_for_signed_out_after_logout, wait_for_sync_stopped,
};
use super::registry::{E2EE_EVENT_TIMEOUT, EVENT_TIMEOUT};
use super::{
    AccountCommand, AccountKey, AuthSecret, CoreCommand, CoreConnection, CoreEvent, CoreRuntime,
    Duration, E2eeTrustEvent, Future, RecoveryRequest, RequestId, SasEmoji, SessionInfo,
    SessionState, SyncCommand, VerificationFlowState, VerificationTarget,
};

pub(super) async fn complete_new_identity_gate_for_qa(
    conn: &mut CoreConnection,
    password: &str,
) -> Result<Option<AuthSecret>, String> {
    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    loop {
        match &conn.snapshot().session {
            SessionState::AwaitingVerification { gate, .. }
                if gate.account_kind == koushi_state::VerificationAccountKind::NewIdentity =>
            {
                break;
            }
            SessionState::Ready(_) => return Ok(None),
            SessionState::Rejecting { .. } => return Err("new identity gate rejected".to_owned()),
            _ => {}
        }
        tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| {
                format!(
                    "timed out waiting for new identity gate; phase={}",
                    gate_session_phase(&conn.snapshot().session)
                )
            })?
            .map_err(|_| "event stream closed while waiting for new identity gate".to_owned())?;
    }
    let request_id = conn.next_request_id();
    let flow_id = request_id.sequence;
    // #1049: the bootstrap reveals its recovery key on screen; no file
    // destination is registered.
    conn.command(CoreCommand::Account(
        AccountCommand::StartSessionBootstrap {
            request_id,
            flow_id,
            auth: Some(AuthSecret::new(password.to_owned())),
            passphrase: Some(AuthSecret::new(password.to_owned())),
        },
    ))
    .await
    .map_err(|error| format!("submit new identity bootstrap: {error}"))?;
    let reveal_deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    // Read the disposable account's key from the live reveal state, the same
    // snapshot projection the WebView renders. It is never printed.
    let recovery_secret = loop {
        let snapshot = conn.snapshot();
        match (
            &snapshot.session,
            &snapshot.e2ee_trust.key_management.secure_backup_setup,
        ) {
            (
                SessionState::AwaitingBootstrapConfirmation {
                    flow_id: active, ..
                },
                koushi_state::SecureBackupSetupState::RecoveryKeyReady {
                    request_id: reveal_id,
                    recovery_key,
                    ..
                },
            ) if *active == flow_id && *reveal_id == flow_id => {
                break AuthSecret::new(recovery_key.expose_secret().to_owned());
            }
            (SessionState::AwaitingVerification { gate, .. }, _) if gate.failure.is_some() => {
                return Err(format!(
                    "new identity bootstrap failed; kind={:?}",
                    gate.failure.expect("failure checked above")
                ));
            }
            _ => {}
        }
        tokio::time::timeout_at(reveal_deadline, conn.recv_event())
            .await
            .map_err(|_| "timed out waiting for bootstrap recovery key reveal".to_owned())?
            .map_err(|_| "event stream closed during bootstrap recovery key reveal".to_owned())?;
    };
    let confirm_id = conn.next_request_id();
    conn.command(CoreCommand::Account(
        AccountCommand::ConfirmSessionBootstrapSaved {
            request_id: confirm_id,
            flow_id,
        },
    ))
    .await
    .map_err(|error| format!("submit bootstrap saved confirmation: {error}"))?;

    // Observe the confirmation's own outcome instead of firing and forgetting
    // it (#375). A failed confirmation leaves the session unpromoted, so
    // `LoggedIn` is never released from the actor's pending-ready events and
    // the run used to surface only `login A: timed out waiting for LoggedIn
    // event` — after this helper had already printed its own success token.
    //
    // Leaving `AwaitingBootstrapConfirmation` is progress, so the loop returns
    // as soon as the session moves on; it deliberately does NOT wait for
    // `Ready`, because the caller owns that wait. Consuming `LoggedIn` here is
    // harmless: the caller resolves the account key from the authoritative
    // snapshot when the session is `Ready`.
    let settle_deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    loop {
        match &conn.snapshot().session {
            SessionState::AwaitingBootstrapConfirmation {
                flow_id: active, ..
            } if *active == flow_id => {}
            SessionState::Rejecting { .. } => {
                return Err("new identity bootstrap confirmation rejected".to_owned());
            }
            _ => break,
        }
        let event = match tokio::time::timeout_at(settle_deadline, conn.recv_event()).await {
            Ok(event) => event,
            Err(_) => {
                return Err(format!(
                    "timed out settling bootstrap confirmation; phase={}",
                    gate_session_phase(&conn.snapshot().session)
                ));
            }
        };
        match event {
            Ok(CoreEvent::OperationFailed {
                request_id: failed,
                failure,
            }) if failed == confirm_id => {
                return Err(format!("bootstrap confirmation failed: {failure:?}"));
            }
            Ok(_) => {}
            Err(_) => continue,
        }
    }

    Ok(Some(recovery_secret))
}

pub(super) async fn wait_for_existing_identity_gate(
    conn: &mut CoreConnection,
    label: &str,
) -> Result<SessionInfo, String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(150);
    loop {
        if let SessionState::AwaitingVerification { info, gate } = &conn.snapshot().session {
            if gate.account_kind == koushi_state::VerificationAccountKind::ExistingIdentity
                && gate
                    .methods
                    .contains(&koushi_state::VerificationMethodCapability::ExistingDeviceSas)
            {
                return Ok(info.clone());
            }
            if gate.account_kind == koushi_state::VerificationAccountKind::ExistingIdentity {
                // Device-list/exact-identity refresh may still be populating
                // the SAS capability. Keep waiting rather than turning this
                // transient gate snapshot into a false prerequisite failure.
            }
        }
        tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| {
                format!(
                    "{label}: timed out; phase={}",
                    gate_session_phase(&conn.snapshot().session)
                )
            })?
            .map_err(|_| format!("{label}: event stream closed"))?;
    }
}

pub(super) async fn wait_for_recovery_gate(
    conn: &mut CoreConnection,
    label: &str,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + E2EE_EVENT_TIMEOUT;
    loop {
        if let SessionState::AwaitingVerification { gate, .. } = &conn.snapshot().session
            && gate.account_kind == koushi_state::VerificationAccountKind::ExistingIdentity
            && gate.methods.iter().any(|method| {
                matches!(
                    method,
                    koushi_state::VerificationMethodCapability::RecoveryKey
                        | koushi_state::VerificationMethodCapability::SecurityPhrase
                )
            })
        {
            return Ok(());
        }
        tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| {
                format!(
                    "{label}: timed out waiting for recovery gate; phase={}",
                    gate_session_phase(&conn.snapshot().session)
                )
            })?
            .map_err(|_| format!("{label}: event stream closed"))?;
    }
}

pub(super) async fn wait_for_matching_recovery_flow(
    conn: &mut CoreConnection,
    flow_id: u64,
    label: &str,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + E2EE_EVENT_TIMEOUT;
    loop {
        if matches!(
            conn.snapshot().session,
            SessionState::Verifying {
                flow_id: active_flow_id,
                method: koushi_state::VerificationMethod::RecoveryKey
                    | koushi_state::VerificationMethod::SecurityPhrase,
                ..
            } if active_flow_id == flow_id
        ) {
            return Ok(());
        }
        tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for matching recovery flow"))?
            .map_err(|_| format!("{label}: event stream closed"))?;
    }
}

pub(super) async fn wait_for_locked_snapshot(
    conn: &mut CoreConnection,
    label: &str,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(150);
    loop {
        if matches!(conn.snapshot().session, SessionState::Locked(_)) {
            return Ok(());
        }
        tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| {
                format!(
                    "{label}: timed out waiting for Locked; phase={}",
                    gate_session_phase(&conn.snapshot().session)
                )
            })?
            .map_err(|_| format!("{label}: event stream closed"))?;
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum SasQaOutcome {
    Success,
    Mismatch,
    UserCancel,
    Timeout,
}

#[derive(Debug, Eq, PartialEq)]
enum SecondarySasObservation {
    Pending,
    Presented(Vec<koushi_state::SasEmoji>),
    Failed,
}

fn observe_secondary_sas(
    session: &SessionState,
    expected_flow_id: u64,
    matching_flow_observed: bool,
) -> SecondarySasObservation {
    match session {
        SessionState::Verifying {
            flow_id,
            sas_emojis,
            ..
        } if *flow_id == expected_flow_id && sas_emojis.len() == 7 => {
            SecondarySasObservation::Presented(sas_emojis.clone())
        }
        SessionState::AwaitingVerification { gate, .. }
            if matching_flow_observed && gate.failure.is_some() =>
        {
            SecondarySasObservation::Failed
        }
        _ => SecondarySasObservation::Pending,
    }
}

pub(super) async fn verify_provisional_second_device_for_qa(
    conn_a: &mut CoreConnection,
    conn_a2: &mut CoreConnection,
    session_a: &SessionInfo,
    session_a2: &SessionInfo,
    label: &str,
    outcome: SasQaOutcome,
) -> Result<(), String> {
    if session_a.user_id != session_a2.user_id || session_a.device_id == session_a2.device_id {
        return Err(format!(
            "{label}: expected two distinct devices for one user"
        ));
    }
    // Keep the primary's normal sync continuously running: it owns incoming
    // to-device delivery for the entire SAS flow, including retry outcomes.
    let previous_primary_flow_id =
        verification_state_flow_id(&conn_a.snapshot().e2ee_trust.verification);
    let target_a2 = VerificationTarget {
        user_id: session_a2.user_id.clone(),
        device_id: session_a2.device_id.clone(),
    };
    let flow_request = conn_a2.next_request_id();
    let flow_id_a2 = flow_request.sequence;
    after_receiver_device_known(
        refresh_device_keys_and_assert_known_for_qa(
            conn_a,
            target_a2.clone(),
            &format!("{label}: primary receiver device discovery"),
        ),
        || async {
            conn_a2
                .command(CoreCommand::Account(AccountCommand::StartOwnUserSas {
                    request_id: flow_request,
                    flow_id: flow_id_a2,
                }))
                .await
                .map_err(|error| format!("{label}: submit own-user SAS: {error}"))
        },
    )
    .await?;

    let flow_id_a = wait_for_verification_requested_event_only(
        conn_a,
        Some(&target_a2),
        previous_primary_flow_id,
        &format!("{label}: primary incoming request"),
    )
    .await?;
    let accept_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Account(AccountCommand::AcceptVerification {
            request_id: accept_id,
            flow_id: flow_id_a,
        }))
        .await
        .map_err(|error| format!("{label}: accept primary: {error}"))?;
    wait_for_verification_accepted(
        conn_a,
        flow_id_a,
        Some(accept_id),
        &format!("{label}: primary ready"),
    )
    .await?;

    let deadline = tokio::time::Instant::now() + E2EE_EVENT_TIMEOUT;
    let mut secondary_matching_flow_observed = false;
    let (emojis_a, emojis_a2) = loop {
        let primary = verification_state_sas(
            &conn_a.snapshot().e2ee_trust.verification,
            flow_id_a,
            &format!("{label}: primary SAS"),
        )?;
        let secondary_session = &conn_a2.snapshot().session;
        secondary_matching_flow_observed |= matches!(
            secondary_session,
            SessionState::Verifying { flow_id, .. } if *flow_id == flow_id_a2
        );
        let secondary = match observe_secondary_sas(
            secondary_session,
            flow_id_a2,
            secondary_matching_flow_observed,
        ) {
            SecondarySasObservation::Presented(emojis) => Some(emojis),
            SecondarySasObservation::Failed => {
                return Err(format!("{label}: secondary gate SAS failed"));
            }
            SecondarySasObservation::Pending => None,
        };
        if let (Some(primary), Some(secondary)) = (primary, secondary) {
            break (primary, secondary);
        }
        match wait_for_paired_event_until(conn_a, conn_a2, deadline).await {
            Ok(()) => {}
            Err(PairedEventWaitError::Deadline) => {
                let primary_snapshot = conn_a.snapshot();
                let (primary_phase, primary_flow_matches, primary_emoji_count) =
                    verification_closed_summary(
                        &primary_snapshot.e2ee_trust.verification,
                        flow_id_a,
                    );
                let secondary_snapshot = conn_a2.snapshot();
                let (secondary_phase, secondary_flow_matches, secondary_emoji_count) =
                    session_gate_closed_summary(&secondary_snapshot.session, flow_id_a2);
                return Err(format!(
                    "{label}: timed out waiting for paired SAS; primary_phase={primary_phase};primary_flow_matches={primary_flow_matches};primary_emoji_count={primary_emoji_count};secondary_phase={secondary_phase};secondary_flow_matches={secondary_flow_matches};secondary_emoji_count={secondary_emoji_count}"
                ));
            }
            Err(PairedEventWaitError::Primary(lag)) => {
                return Err(verification_event_stream_error(label, "primary", lag));
            }
            Err(PairedEventWaitError::Secondary(lag)) => {
                return Err(verification_event_stream_error(label, "secondary", lag));
            }
        }
    };
    if emojis_a != emojis_a2 {
        return Err(format!("{label}: SAS emoji mismatch"));
    }
    if outcome == SasQaOutcome::Timeout {
        wait_for_existing_identity_gate(conn_a2, &format!("{label}: timeout retryable")).await?;
        return Ok(());
    }
    if outcome != SasQaOutcome::Success {
        let cancel_a2 = conn_a2.next_request_id();
        conn_a2
            .command(CoreCommand::Account(AccountCommand::CancelVerification {
                request_id: cancel_a2,
                flow_id: flow_id_a2,
                reason: if outcome == SasQaOutcome::Mismatch {
                    koushi_state::VerificationCancelReason::Mismatch
                } else {
                    koushi_state::VerificationCancelReason::User
                },
            }))
            .await
            .map_err(|error| format!("{label}: mismatch secondary: {error}"))?;
        let cancel_a = conn_a.next_request_id();
        conn_a
            .command(CoreCommand::Account(AccountCommand::CancelVerification {
                request_id: cancel_a,
                flow_id: flow_id_a,
                reason: koushi_state::VerificationCancelReason::User,
            }))
            .await
            .map_err(|error| format!("{label}: cancel primary after mismatch: {error}"))?;
        wait_for_existing_identity_gate(conn_a2, &format!("{label}: mismatch retryable")).await?;
        return Ok(());
    }

    let confirm_a = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Account(
            AccountCommand::ConfirmSasVerification {
                request_id: confirm_a,
                flow_id: flow_id_a,
            },
        ))
        .await
        .map_err(|error| format!("{label}: confirm primary: {error}"))?;
    let confirm_a2 = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Account(
            AccountCommand::ConfirmSasVerification {
                request_id: confirm_a2,
                flow_id: flow_id_a2,
            },
        ))
        .await
        .map_err(|error| format!("{label}: confirm secondary: {error}"))?;

    let ready_deadline = tokio::time::Instant::now() + E2EE_EVENT_TIMEOUT;
    loop {
        if matches!(conn_a2.snapshot().session, SessionState::Ready(_)) {
            return Ok(());
        }
        match (QaEventDeadline {
            instant: ready_deadline,
        })
        .recv(conn_a2)
        .await
        {
            Ok(Ok(_)) => {}
            Ok(Err(lag)) => {
                return Err(verification_event_stream_error(label, "secondary", lag));
            }
            Err(_) => {
                return Err(format!(
                    "{label}: timed out waiting for authoritative Ready"
                ));
            }
        }
    }
}

async fn after_receiver_device_known<Refresh, Start, Started, Output, Error>(
    refresh: Refresh,
    start_once: Start,
) -> Result<Output, Error>
where
    Refresh: Future<Output = Result<(), Error>>,
    Start: FnOnce() -> Started,
    Started: Future<Output = Result<Output, Error>>,
{
    refresh.await?;
    start_once().await
}

pub(super) fn authenticated_session_info(
    conn: &mut CoreConnection,
    label: &str,
) -> Result<SessionInfo, String> {
    authenticated_session_info_from_state(&conn.snapshot().session)
        .cloned()
        .ok_or_else(|| format!("{label}: session is not authenticated"))
}

pub(super) fn authenticated_session_info_from_state(
    session: &SessionState,
) -> Option<&SessionInfo> {
    match session {
        SessionState::Provisional { info, .. }
        | SessionState::AwaitingVerification { info, .. }
        | SessionState::Verifying { info, .. }
        | SessionState::AwaitingBootstrapConfirmation { info, .. }
        | SessionState::Rejecting { info, .. }
        | SessionState::Ready(info) => Some(info),
        SessionState::SignedOut
        | SessionState::Restoring
        | SessionState::SwitchingAccount { .. }
        | SessionState::Authenticating { .. }
        | SessionState::Locked(_)
        | SessionState::CapabilityBlocked { .. }
        | SessionState::LoggingOut => None,
    }
}

pub(super) async fn refresh_device_keys_and_assert_known_for_qa(
    conn: &mut CoreConnection,
    target: VerificationTarget,
    label: &str,
) -> Result<(), String> {
    tokio::time::timeout(
        E2EE_EVENT_TIMEOUT,
        conn.qa_refresh_device_keys_and_assert_known(target),
    )
    .await
    .map_err(|_| format!("{label}: timed out waiting for device-key refresh checkpoint"))?
    .map_err(|_| format!("{label}: exact device was not known after key refresh"))
}

pub(super) enum QaParticipantLoginGate<'a> {
    BootstrapNewIdentity,
    RecoverExistingIdentity(&'a AuthSecret),
}

pub(super) struct QaParticipantLoginOutcome {
    pub(super) runtime: CoreRuntime,
    pub(super) conn: CoreConnection,
    pub(super) account_key: AccountKey,
    pub(super) bootstrap_recovery_secret: Option<AuthSecret>,
}

pub(super) struct QaOwnedLoggedInRuntime {
    pub(super) runtime: CoreRuntime,
    pub(super) conn: CoreConnection,
    pub(super) account_key: AccountKey,
}

impl From<QaParticipantLoginOutcome> for QaOwnedLoggedInRuntime {
    fn from(participant: QaParticipantLoginOutcome) -> Self {
        Self {
            runtime: participant.runtime,
            conn: participant.conn,
            account_key: participant.account_key,
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub(super) enum QaOwnedRuntimePhase {
    LoginNotSubmitted,
    LoginSubmitted,
    LoggedIn(AccountKey),
}

pub(super) struct QaOwnedRuntimeParticipant {
    pub(super) runtime: CoreRuntime,
    pub(super) conn: CoreConnection,
    pub(super) phase: QaOwnedRuntimePhase,
}

impl QaOwnedRuntimeParticipant {
    pub(super) fn new(runtime: CoreRuntime, conn: CoreConnection) -> Self {
        Self {
            runtime,
            conn,
            phase: QaOwnedRuntimePhase::LoginNotSubmitted,
        }
    }

    pub(super) fn from_logged_in(participant: QaOwnedLoggedInRuntime) -> Self {
        Self {
            runtime: participant.runtime,
            conn: participant.conn,
            phase: QaOwnedRuntimePhase::LoggedIn(participant.account_key),
        }
    }

    pub(super) fn mark_login_submitted(&mut self) {
        self.phase = QaOwnedRuntimePhase::LoginSubmitted;
    }

    pub(super) fn mark_logged_in(&mut self, account_key: AccountKey) {
        self.phase = QaOwnedRuntimePhase::LoggedIn(account_key);
    }

    fn logged_in_connection_and_account_key(
        &mut self,
    ) -> Option<(&mut CoreConnection, &AccountKey)> {
        let QaOwnedRuntimePhase::LoggedIn(account_key) = &self.phase else {
            return None;
        };
        Some((&mut self.conn, account_key))
    }

    pub(super) fn into_logged_in_runtime(self) -> QaOwnedLoggedInRuntime {
        let QaOwnedRuntimePhase::LoggedIn(account_key) = self.phase else {
            panic!("caller ownership returns only after a completed login");
        };
        QaOwnedLoggedInRuntime {
            runtime: self.runtime,
            conn: self.conn,
            account_key,
        }
    }
}

impl From<QaParticipantLoginOutcome> for QaOwnedRuntimeParticipant {
    fn from(participant: QaParticipantLoginOutcome) -> Self {
        Self::from_logged_in(QaOwnedLoggedInRuntime::from(participant))
    }
}

#[expect(
    clippy::large_enum_variant,
    reason = "QA recipient selector built once per scenario step; boxing adds churn without a measurable benefit"
)]
pub(super) enum QaE2eeRecipient<'a> {
    Borrowed {
        conn: &'a mut CoreConnection,
        account_key: &'a AccountKey,
    },
    Owned(QaOwnedRuntimeParticipant),
}

impl QaE2eeRecipient<'_> {
    pub(super) fn connection_and_account_key(&mut self) -> (&mut CoreConnection, &AccountKey) {
        match self {
            Self::Borrowed { conn, account_key } => (conn, account_key),
            Self::Owned(participant) => participant
                .logged_in_connection_and_account_key()
                .expect("owned E2EE recipient login completed before the post-login stage"),
        }
    }

    pub(super) fn into_owned(self) -> Option<QaOwnedRuntimeParticipant> {
        match self {
            Self::Borrowed { .. } => None,
            Self::Owned(participant) => Some(participant),
        }
    }
}

pub(super) async fn finish_e2ee_recipient_stage_with_owned_cleanup<
    T,
    Participant,
    Cleanup,
    CleanupFuture,
>(
    stage_result: Result<T, String>,
    owned_participant: Option<Participant>,
    cleanup: Cleanup,
) -> Result<T, String>
where
    Cleanup: FnOnce(Participant) -> CleanupFuture,
    CleanupFuture: Future<Output = Result<(), String>>,
{
    let cleanup_result = match owned_participant {
        Some(participant) => cleanup(participant).await,
        None => Ok(()),
    };

    match (stage_result, cleanup_result) {
        (Err(stage_error), Ok(())) => Err(stage_error),
        (Err(stage_error), Err(_)) => Err(format!(
            "{stage_error}; owned E2EE recipient cleanup also failed"
        )),
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(cleanup_error)) => Err(cleanup_error),
    }
}

pub(super) async fn retain_or_cleanup_e2ee_callers_after_stage<Callers, Cleanup, CleanupFuture>(
    stage_result: Result<(), String>,
    callers: Callers,
    cleanup: Cleanup,
) -> Result<Callers, String>
where
    Cleanup: FnOnce(Callers) -> CleanupFuture,
    CleanupFuture: Future<Output = Result<(), String>>,
{
    match stage_result {
        Ok(()) => Ok(callers),
        Err(stage_error) => match cleanup(callers).await {
            Ok(()) => Err(stage_error),
            Err(_) => Err(format!("{stage_error}; E2EE caller cleanup also failed")),
        },
    }
}

async fn retain_or_cleanup_owned_participant_after_stage<T, Participant, Cleanup, CleanupFuture>(
    stage_result: Result<T, String>,
    participant: Participant,
    cleanup: Cleanup,
) -> Result<(T, Participant), String>
where
    Cleanup: FnOnce(Participant) -> CleanupFuture,
    CleanupFuture: Future<Output = Result<(), String>>,
{
    match stage_result {
        Ok(value) => Ok((value, participant)),
        Err(stage_error) => match cleanup(participant).await {
            Ok(()) => Err(stage_error),
            Err(_) => Err(format!(
                "{stage_error}; owned participant login cleanup also failed"
            )),
        },
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "pre-existing internal signature; grouping its inputs is a separate refactor"
)]
pub(super) async fn login_synced_participant_for_qa(
    homeserver: &str,
    data_dir: std::path::PathBuf,
    username: &str,
    password: &str,
    device_display_name: &str,
    label: &str,
    gate_label: &str,
    gate: QaParticipantLoginGate<'_>,
) -> Result<QaParticipantLoginOutcome, String> {
    let runtime = CoreRuntime::start_with_data_dir(data_dir);
    let conn = runtime.attach();
    let mut participant = QaOwnedRuntimeParticipant::new(runtime, conn);
    let login_stage_result: Result<Option<AuthSecret>, String> = async {
        let login_id = participant.conn.next_request_id();
        participant
            .conn
            .command(CoreCommand::Account(AccountCommand::LoginPassword {
                request_id: login_id,
                request: koushi_state::LoginRequest {
                    homeserver: homeserver.to_owned(),
                    username: username.to_owned(),
                    password: AuthSecret::new(password.to_owned()),
                    device_display_name: Some(device_display_name.to_owned()),
                },
                platform: koushi_state::DisplayPlatform::Linux,
            }))
            .await
            .map_err(|e| format!("{label}: submit login failed: {e}"))?;
        participant.mark_login_submitted();
        let bootstrap_recovery_secret = match gate {
            QaParticipantLoginGate::BootstrapNewIdentity => {
                complete_new_identity_gate_for_qa(&mut participant.conn, password).await?
            }
            QaParticipantLoginGate::RecoverExistingIdentity(recovery_secret) => {
                wait_for_recovery_gate(&mut participant.conn, gate_label).await?;
                let recovery_request_id = participant.conn.next_request_id();
                participant
                    .conn
                    .command(CoreCommand::Account(AccountCommand::SubmitRecovery {
                        request_id: recovery_request_id,
                        request: RecoveryRequest {
                            secret: recovery_secret.clone(),
                        },
                    }))
                    .await
                    .map_err(|e| format!("{gate_label}: submit recovery failed: {e}"))?;
                None
            }
        };
        let account_key = wait_for_logged_in(&mut participant.conn, login_id, label).await?;
        participant.mark_logged_in(account_key);
        wait_for_ready_snapshot(&mut participant.conn, label).await?;
        start_sync_for_qa(&mut participant.conn, label).await?;
        Ok(bootstrap_recovery_secret)
    }
    .await;
    let (bootstrap_recovery_secret, participant) = retain_or_cleanup_owned_participant_after_stage(
        login_stage_result,
        participant,
        |participant| async move {
            cleanup_owned_e2ee_participant_best_effort(participant, "participant login cleanup")
                .await
        },
    )
    .await?;
    let QaOwnedLoggedInRuntime {
        runtime,
        conn,
        account_key,
    } = participant.into_logged_in_runtime();

    Ok(QaParticipantLoginOutcome {
        runtime,
        conn,
        account_key,
        bootstrap_recovery_secret,
    })
}

pub(super) fn ensure_incoming_verification_receiver_sync_not_stopped(
    sync: &koushi_state::SyncState,
    label: &str,
) -> Result<(), String> {
    if matches!(sync, koushi_state::SyncState::Stopped) {
        Err(format!(
            "{label}: receiver sync is stopped; cannot await an incoming verification request"
        ))
    } else {
        Ok(())
    }
}

pub(super) async fn wait_for_verification_requested_event_only(
    conn: &mut CoreConnection,
    expected_target: Option<&VerificationTarget>,
    excluded_flow_id: Option<u64>,
    label: &str,
) -> Result<u64, String> {
    ensure_incoming_verification_receiver_sync_not_stopped(&conn.snapshot().sync, label)?;
    let deadline = tokio::time::Instant::now() + E2EE_EVENT_TIMEOUT;

    loop {
        if let Some(flow_id) = requested_verification_flow_id(
            &conn.snapshot().e2ee_trust.verification,
            expected_target,
            excluded_flow_id,
        )? {
            return Ok(flow_id);
        }

        let event = tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for incoming verification request"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;
        if let CoreEvent::E2eeTrust(E2eeTrustEvent::VerificationProgress { state, .. }) = event
            && let Some(flow_id) =
                requested_verification_flow_id(&state, expected_target, excluded_flow_id)?
        {
            return Ok(flow_id);
        }
    }
}

pub(super) fn requested_verification_flow_id(
    state: &VerificationFlowState,
    expected_target: Option<&VerificationTarget>,
    excluded_flow_id: Option<u64>,
) -> Result<Option<u64>, String> {
    if verification_state_flow_id(state).is_some_and(|flow_id| Some(flow_id) == excluded_flow_id) {
        return Ok(None);
    }
    if !verification_state_matches_target(state, expected_target) {
        return Ok(None);
    }

    match state {
        VerificationFlowState::Requested { request_id, .. }
        | VerificationFlowState::Accepted { request_id, .. }
        | VerificationFlowState::SasPresented { request_id, .. }
        | VerificationFlowState::Confirming { request_id, .. }
        | VerificationFlowState::Done { request_id, .. } => Ok(Some(*request_id)),
        VerificationFlowState::Failed { kind, .. } => Err(format!(
            "verification request failed before acceptance: {kind:?}"
        )),
        VerificationFlowState::Idle => Ok(None),
    }
}

pub(super) async fn wait_for_verification_accepted(
    conn: &mut CoreConnection,
    flow_id: u64,
    command_request_id: Option<RequestId>,
    label: &str,
) -> Result<(), String> {
    if verification_state_is_at_least_accepted(&conn.snapshot().e2ee_trust.verification, flow_id)? {
        return Ok(());
    }

    let deadline = QaEventDeadline::after(E2EE_EVENT_TIMEOUT);
    loop {
        let event = deadline
            .recv(conn)
            .await
            .map_err(|_| format!("{label}: timed out waiting for verification acceptance"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::E2eeTrust(E2eeTrustEvent::VerificationProgress { state, .. }) => {
                if verification_state_is_at_least_accepted(&state, flow_id)? {
                    return Ok(());
                }
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if command_request_id == Some(ev_id) => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

pub(super) fn verification_state_is_at_least_accepted(
    state: &VerificationFlowState,
    flow_id: u64,
) -> Result<bool, String> {
    if verification_state_flow_id(state) != Some(flow_id) {
        return Ok(false);
    }
    match state {
        VerificationFlowState::Accepted { .. }
        | VerificationFlowState::SasPresented { .. }
        | VerificationFlowState::Confirming { .. }
        | VerificationFlowState::Done { .. } => Ok(true),
        VerificationFlowState::Failed { kind, .. } => {
            Err(format!("verification failed before acceptance: {kind:?}"))
        }
        VerificationFlowState::Idle | VerificationFlowState::Requested { .. } => Ok(false),
    }
}

pub(super) fn verification_state_sas(
    state: &VerificationFlowState,
    flow_id: u64,
    label: &str,
) -> Result<Option<Vec<SasEmoji>>, String> {
    if verification_state_flow_id(state) != Some(flow_id) {
        return Ok(None);
    }
    match state {
        VerificationFlowState::SasPresented { emojis, .. }
        | VerificationFlowState::Confirming { emojis, .. } => Ok(Some(emojis.clone())),
        VerificationFlowState::Done { .. } => Err(format!(
            "{label}: verification completed before SAS was observed"
        )),
        VerificationFlowState::Failed { kind, .. } => {
            Err(format!("{label}: verification failed before SAS: {kind:?}"))
        }
        VerificationFlowState::Idle
        | VerificationFlowState::Requested { .. }
        | VerificationFlowState::Accepted { .. } => Ok(None),
    }
}

fn verification_state_matches_target(
    state: &VerificationFlowState,
    expected_target: Option<&VerificationTarget>,
) -> bool {
    expected_target.is_none_or(|target| verification_state_target(state) == Some(target))
}

/// Data directory for QA runs.
pub(super) fn qa_data_dir(suffix: &str) -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("KOUSHI_QA_DATA_DIR") {
        return std::path::PathBuf::from(dir).join(suffix);
    }
    std::env::temp_dir()
        .join("koushi-core-qa")
        .join(format!("{}_{}", std::process::id(), suffix))
}

/// Start a same-user secondary-device fixture without restoring the primary
/// device's saved session from the process-wide QA credential store.
pub(super) fn start_isolated_qa_runtime(suffix: &str) -> Result<CoreRuntime, String> {
    let credential_root = std::env::var_os(super::registry::ENV_FILE_CREDENTIAL_STORE_DIR)
        .ok_or_else(|| "QA file credential store directory is not configured".to_owned())?;
    Ok(CoreRuntime::start_with_data_dir_and_file_credentials(
        qa_data_dir(suffix),
        std::path::PathBuf::from(credential_root)
            .join("isolated-devices")
            .join(suffix),
    ))
}

#[cfg(test)]
#[path = "participants_tests.rs"]
mod tests;

fn verification_state_target(state: &VerificationFlowState) -> Option<&VerificationTarget> {
    match state {
        VerificationFlowState::Idle => None,
        VerificationFlowState::Requested { target, .. }
        | VerificationFlowState::Accepted { target, .. }
        | VerificationFlowState::SasPresented { target, .. }
        | VerificationFlowState::Confirming { target, .. }
        | VerificationFlowState::Done { target, .. }
        | VerificationFlowState::Failed { target, .. } => Some(target),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum QaE2eeLogoutBarrier {
    AnyAccount,
    Exact(AccountKey),
}

fn e2ee_cleanup_logout_barrier(phase: &QaOwnedRuntimePhase) -> Option<QaE2eeLogoutBarrier> {
    match phase {
        QaOwnedRuntimePhase::LoginNotSubmitted => None,
        // Login was submitted, but ownership has not advanced through the
        // authoritative LoggedIn gate. Do not infer an exact account key from
        // a provisional snapshot.
        QaOwnedRuntimePhase::LoginSubmitted => Some(QaE2eeLogoutBarrier::AnyAccount),
        QaOwnedRuntimePhase::LoggedIn(account_key) => {
            Some(QaE2eeLogoutBarrier::Exact(account_key.clone()))
        }
    }
}

pub(super) trait QaOwnedE2eeCleanupOperations {
    async fn stop_sync(&mut self, label: &str) -> Result<(), String>;
    async fn submit_logout(
        &mut self,
        barrier: &QaE2eeLogoutBarrier,
        label: &str,
    ) -> Result<(), String>;
    async fn wait_for_authoritative_logout(
        &mut self,
        barrier: &QaE2eeLogoutBarrier,
        label: &str,
    ) -> Result<(), String>;
    fn drop_connection(&mut self);
    async fn shutdown_runtime(&mut self);
}

struct QaCoreOwnedE2eeCleanupOperations {
    runtime: Option<CoreRuntime>,
    conn: Option<CoreConnection>,
    logout_request_id: Option<koushi_protocol::ids::RequestId>,
}

impl QaCoreOwnedE2eeCleanupOperations {
    fn new(runtime: CoreRuntime, conn: CoreConnection) -> Self {
        Self {
            runtime: Some(runtime),
            conn: Some(conn),
            logout_request_id: None,
        }
    }

    fn connection(&mut self) -> &mut CoreConnection {
        self.conn
            .as_mut()
            .expect("owned E2EE cleanup connection is available before its drop barrier")
    }
}

impl QaOwnedE2eeCleanupOperations for QaCoreOwnedE2eeCleanupOperations {
    async fn stop_sync(&mut self, label: &str) -> Result<(), String> {
        let conn = self.connection();
        let sync_stop_id = conn.next_request_id();
        match conn
            .command(CoreCommand::Sync(SyncCommand::Stop {
                request_id: sync_stop_id,
            }))
            .await
        {
            Ok(()) => wait_for_sync_stopped(conn, sync_stop_id, label).await,
            Err(_) => Err(format!("{label}: submit sync stop failed")),
        }
    }

    async fn submit_logout(
        &mut self,
        _barrier: &QaE2eeLogoutBarrier,
        label: &str,
    ) -> Result<(), String> {
        let conn = self.connection();
        let logout_request_id = conn.next_request_id();
        conn.command(CoreCommand::Account(AccountCommand::Logout {
            request_id: logout_request_id,
        }))
        .await
        .map_err(|_| format!("{label}: submit logout failed"))?;
        self.logout_request_id = Some(logout_request_id);
        Ok(())
    }

    async fn wait_for_authoritative_logout(
        &mut self,
        barrier: &QaE2eeLogoutBarrier,
        label: &str,
    ) -> Result<(), String> {
        let logout_request_id = self
            .logout_request_id
            .take()
            .expect("logout submission precedes its authoritative cleanup barrier");
        let conn = self.connection();
        match barrier {
            QaE2eeLogoutBarrier::AnyAccount => {
                wait_for_signed_out_after_logout(conn, logout_request_id, label).await
            }
            QaE2eeLogoutBarrier::Exact(account_key) => {
                wait_for_logged_out(conn, logout_request_id, account_key, label).await
            }
        }
    }

    fn drop_connection(&mut self) {
        drop(self.conn.take());
    }

    async fn shutdown_runtime(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown().await;
        }
    }
}

pub(super) async fn cleanup_owned_e2ee_lifecycle_best_effort<Operations>(
    phase: &QaOwnedRuntimePhase,
    operations: &mut Operations,
    label: &str,
) -> Result<(), String>
where
    Operations: QaOwnedE2eeCleanupOperations,
{
    let sync_stop_result = if matches!(phase, QaOwnedRuntimePhase::LoggedIn(_)) {
        operations.stop_sync(label).await
    } else {
        Ok(())
    };

    // Logout is attempted even if stopping sync failed. Connection drop and
    // ordered runtime shutdown remain the final barriers in every phase.
    let logout_result = if let Some(barrier) = e2ee_cleanup_logout_barrier(phase) {
        match operations.submit_logout(&barrier, label).await {
            Ok(()) => {
                operations
                    .wait_for_authoritative_logout(&barrier, label)
                    .await
            }
            Err(error) => Err(error),
        }
    } else {
        Ok(())
    };

    operations.drop_connection();
    operations.shutdown_runtime().await;

    match (sync_stop_result, logout_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(_), Ok(())) => Err(format!("{label}: sync stop cleanup failed")),
        (Ok(()), Err(_)) => Err(format!("{label}: logout cleanup failed")),
        (Err(_), Err(_)) => Err(format!("{label}: sync stop and logout cleanup failed")),
    }
}

pub(super) async fn cleanup_owned_e2ee_participant_best_effort(
    participant: QaOwnedRuntimeParticipant,
    label: &str,
) -> Result<(), String> {
    let QaOwnedRuntimeParticipant {
        runtime,
        conn,
        phase,
    } = participant;
    let mut operations = QaCoreOwnedE2eeCleanupOperations::new(runtime, conn);
    cleanup_owned_e2ee_lifecycle_best_effort(&phase, &mut operations, label).await
}
