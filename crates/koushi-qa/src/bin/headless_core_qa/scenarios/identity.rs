use super::cleanup::{cleanup_e2ee_multi_device_participants, leave_e2ee_login_store_room};
use super::event_wait::{
    QaEventDeadline, find_timeline_item_with_body, start_sync_for_qa, subscribe_timeline_for_qa,
    timeline_item_is_decryption_failure, wait_for_initial_items, wait_for_invite_in_snapshot,
    wait_for_item_with_body_or_decryption_failure, wait_for_logged_in, wait_for_operation_failed,
    wait_for_operation_failed_and_signed_out, wait_for_ready_snapshot, wait_for_room_created,
    wait_for_room_in_room_list, wait_for_send_flow_completion,
    wait_for_send_flow_completion_with_timeout, wait_for_session_restored,
    wait_for_sync_started_and_running, wait_for_sync_stopped,
    wait_for_withheld_event_projection_from_source,
};
use super::fixtures::{
    accept_invite_for_qa, assert_room_settings_contains_members, create_room_for_qa,
    invite_user_for_qa, load_room_settings_for_qa, native_attention_room, private_room_options,
};
use super::participants::{
    QaE2eeRecipient, QaOwnedRuntimeParticipant, QaParticipantLoginGate, SasQaOutcome,
    authenticated_session_info, cleanup_owned_e2ee_participant_best_effort,
    finish_e2ee_recipient_stage_with_owned_cleanup, login_synced_participant_for_qa, qa_data_dir,
    refresh_device_keys_and_assert_known_for_qa, start_isolated_qa_runtime,
    verify_provisional_second_device_for_qa, wait_for_existing_identity_gate,
    wait_for_locked_snapshot, wait_for_matching_recovery_flow, wait_for_recovery_gate,
};
use super::registry::{
    DEVICE_A, DEVICE_B, E2EE_EVENT_TIMEOUT, E2EE_KEY_BACKUP_SEED_BODY,
    E2EE_MULTI_USER_MULTI_DEVICE_BODY, E2EE_SECOND_DEVICE_BODY, ENV_E2EE_RECIPIENT_SECOND_DEVICE,
    EVENT_TIMEOUT, GATE_RESTORE_READY_BUDGET, QA_WRONG_RECOVERY_SECRET, QaConfig, env_flag_enabled,
};
use super::{
    AccountCommand, AccountEvent, AccountKey, AppCommand, AuthSecret, CoreCommand, CoreConnection,
    CoreEvent, CoreFailure, CoreRuntime, CurrentSessionStatusState, CurrentSessionSyncState,
    DeviceCleanupLocalMode, DeviceCleanupState, Duration, E2eeTrustEvent, IdentityResetAuthRequest,
    IdentityResetAuthType, IdentityResetState, KeyBackupStatus, LocalEncryptionEvent,
    LocalEncryptionHealth, LocalEncryptionState, NativeAttentionCapabilities,
    NativeAttentionCapability, NativeAttentionDispatchState, NativeAttentionObservationKind,
    NativeAttentionProjectionInput, NativeAttentionState, NativeAttentionSuppressionReason,
    RecoveryRequest, RequestId, RoomAttentionKind, RoomCommand, RoomEvent, RoomNotificationMode,
    SessionAuthenticationMethod, SessionInfo, SessionState, SessionStatusRefreshTrigger,
    SyncCommand, TimelineCommand, TimelineItem, TimelineKey, VerificationTarget,
    native_attention_state_from_rooms,
};

#[derive(Clone)]
struct StoppedQaParticipant {
    data_dir: std::path::PathBuf,
    account_key: AccountKey,
    device_id: String,
}

async fn restart_stopped_qa_participant(
    stopped: StoppedQaParticipant,
    label: &str,
) -> Result<QaOwnedRuntimeParticipant, String> {
    let runtime = CoreRuntime::start_with_data_dir(stopped.data_dir);
    let conn = runtime.attach();
    let mut participant = QaOwnedRuntimeParticipant::new(runtime, conn);
    let result: Result<(), String> = async {
        let restore_id = participant.conn.next_request_id();
        participant.mark_login_submitted();
        participant
            .conn
            .command(CoreCommand::Account(AccountCommand::RestoreSession {
                request_id: restore_id,
                account_key: stopped.account_key.clone(),
            }))
            .await
            .map_err(|_| format!("{label}: submit store restore failed"))?;
        wait_for_session_restored(
            &mut participant.conn,
            restore_id,
            &stopped.account_key,
            label,
        )
        .await?;
        participant.mark_logged_in(stopped.account_key.clone());
        wait_for_ready_snapshot(&mut participant.conn, label).await?;
        let restored_info = authenticated_session_info(&mut participant.conn, label)?;
        if restored_info.device_id != stopped.device_id {
            return Err(format!("{label}: restored device identity changed"));
        }
        start_sync_for_qa(&mut participant.conn, label).await
    }
    .await;

    match result {
        Ok(()) => Ok(participant),
        Err(error) => {
            let _ = cleanup_owned_e2ee_participant_best_effort(participant, label).await;
            Err(error)
        }
    }
}

async fn stop_qa_participant_for_offline(
    participant: QaOwnedRuntimeParticipant,
    data_dir: std::path::PathBuf,
    account_key: AccountKey,
    device_id: String,
) -> Result<StoppedQaParticipant, String> {
    let QaOwnedRuntimeParticipant {
        runtime, mut conn, ..
    } = participant;
    let stop_id = conn.next_request_id();
    conn.command(CoreCommand::Sync(SyncCommand::Stop {
        request_id: stop_id,
    }))
    .await
    .map_err(|_| "offline participant sync stop submit failed".to_owned())?;
    wait_for_sync_stopped(&mut conn, stop_id, "offline participant sync stop").await?;
    drop(conn);
    runtime.shutdown().await;
    Ok(StoppedQaParticipant {
        data_dir,
        account_key,
        device_id,
    })
}

async fn send_after_rotation(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    room_id: &str,
    body: &str,
    transaction_id: &str,
    deadline: tokio::time::Instant,
    label: &str,
) -> Result<(), String> {
    let key = TimelineKey::room(account_key.clone(), room_id.to_owned());
    let send_id = conn.next_request_id();
    tokio::time::timeout_at(
        deadline,
        conn.command(CoreCommand::Timeline(TimelineCommand::SendText {
            request_id: send_id,
            key: key.clone(),
            transaction_id: transaction_id.to_owned(),
            document: koushi_state::ComposerDocument::from_plain_text(body.to_owned()),
        })),
    )
    .await
    .map_err(|_| format!("{label}: send submit timed out"))?
    .map_err(|_| format!("{label}: send submit failed"))?;
    tokio::time::timeout_at(
        deadline,
        wait_for_send_flow_completion_with_timeout(
            conn,
            send_id,
            &key,
            transaction_id,
            body,
            label,
            E2EE_EVENT_TIMEOUT,
        ),
    )
    .await
    .map_err(|_| format!("{label}: send completion timed out"))??;
    Ok(())
}

async fn force_rotate_outbound_session(
    conn: &mut CoreConnection,
    room_id: &str,
    deadline: tokio::time::Instant,
) -> Result<(), String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::ForceRotateOutboundSession {
        request_id,
        room_id: room_id.to_owned(),
    }))
    .await
    .map_err(|_| "e2ee login-store forced rotation submit failed".to_owned())?;
    loop {
        match (QaEventDeadline { instant: deadline }).recv(conn).await {
            Ok(Ok(CoreEvent::Room(RoomEvent::OutboundSessionRotationForced {
                request_id: event_request_id,
                ..
            }))) if event_request_id == request_id => return Ok(()),
            Ok(Ok(CoreEvent::OperationFailed {
                request_id: event_request_id,
                ..
            })) if event_request_id == request_id => {
                return Err("e2ee login-store forced rotation failed".to_owned());
            }
            Ok(Ok(_)) => {}
            Ok(Err(_)) => return Err("e2ee login-store forced rotation event lagged".to_owned()),
            Err(_) => return Err("e2ee login-store forced rotation timed out".to_owned()),
        }
    }
}

async fn assert_inbound_sessions_start_at_zero(
    conn: &CoreConnection,
    room_id: &str,
    label: &str,
) -> Result<usize, String> {
    tokio::time::timeout(
        E2EE_EVENT_TIMEOUT,
        conn.qa_assert_inbound_sessions_start_at_zero(room_id.to_owned()),
    )
    .await
    .map_err(|_| format!("{label}: inbound-session index assertion timed out"))?
    .map_err(|_| format!("{label}: an inbound session did not start at index 0"))
}

async fn restart_recipient(
    recipient: &mut Option<QaOwnedRuntimeParticipant>,
    stopped: &mut Option<StoppedQaParticipant>,
    label: &str,
    deadline: tokio::time::Instant,
) -> Result<(), String> {
    let stopped_copy = stopped
        .clone()
        .ok_or_else(|| format!("{label}: recipient was not stopped"))?;
    let restarted = tokio::time::timeout_at(
        deadline,
        restart_stopped_qa_participant(stopped_copy, label),
    )
    .await
    .map_err(|_| format!("{label}: recipient restart timed out"))??;
    *recipient = Some(restarted);
    *stopped = None;
    Ok(())
}

pub(super) async fn run_e2ee_login_store_scenario(config: &QaConfig) -> Result<(), String> {
    let data_dir_a = qa_data_dir("e2ee-login-store-a");
    let data_dir_b = qa_data_dir("e2ee-login-store-b");
    let mut a_outcome = login_synced_participant_for_qa(
        &config.homeserver,
        data_dir_a.clone(),
        &config.user_a,
        &config.password_a,
        DEVICE_A,
        "e2ee login-store A",
        "e2ee login-store bootstrap A",
        QaParticipantLoginGate::BootstrapNewIdentity,
    )
    .await?;
    let a_info = authenticated_session_info(&mut a_outcome.conn, "e2ee login-store A session")?;
    let a_device_id = a_info.device_id.clone();
    let a_account_key = a_outcome.account_key.clone();
    let mut owner_a = Some(QaOwnedRuntimeParticipant::from(a_outcome));

    let mut b_outcome = login_synced_participant_for_qa(
        &config.homeserver,
        data_dir_b.clone(),
        &config.user_b,
        &config.password_b,
        DEVICE_B,
        "e2ee login-store B",
        "e2ee login-store bootstrap B",
        QaParticipantLoginGate::BootstrapNewIdentity,
    )
    .await?;
    let b_info = authenticated_session_info(&mut b_outcome.conn, "e2ee login-store B session")?;
    let b_device_id = b_info.device_id.clone();
    let b_account_key = b_outcome.account_key.clone();
    let mut owner_b = Some(QaOwnedRuntimeParticipant::from(b_outcome));
    let mut stopped_b = None;
    let mut stopped_a = None;
    let mut owner_c = None;
    let mut rooms = Vec::new();

    let stage_result: Result<(), String> = async {
        let user_b_id = format!("@{}:{}", config.user_b, config.server_name);
        let a = owner_a.as_mut().ok_or("e2ee login-store A owner missing")?;
        let room_id = create_room_for_qa(
            &mut a.conn,
            "QA E2EE login store DM",
            true,
            "e2ee login-store create DM",
        )
        .await?;
        rooms.push(room_id.clone());
        invite_user_for_qa(
            &mut a.conn,
            &room_id,
            &user_b_id,
            "e2ee login-store invite B",
        )
        .await?;
        let b = owner_b.as_mut().ok_or("e2ee login-store B owner missing")?;
        wait_for_invite_in_snapshot(
            &mut b.conn,
            &room_id,
            Some(false),
            "e2ee login-store B invite",
        )
        .await?;
        accept_invite_for_qa(&mut b.conn, &room_id, "e2ee login-store B accept").await?;
        wait_for_room_in_room_list(&mut a.conn, &room_id, "e2ee login-store A room").await?;
        wait_for_room_in_room_list(&mut b.conn, &room_id, "e2ee login-store B room").await?;

        let b_session = authenticated_session_info(&mut b.conn, "e2ee login-store B identity")?;
        refresh_device_keys_and_assert_known_for_qa(
            &mut a.conn,
            VerificationTarget {
                user_id: b_session.user_id.clone(),
                device_id: b_session.device_id.clone(),
            },
            "e2ee login-store B key refresh",
        )
        .await?;
        let key_a = TimelineKey::room(a_account_key.clone(), room_id.clone());
        let key_b = TimelineKey::room(b_account_key.clone(), room_id.clone());
        let initial_a =
            subscribe_timeline_for_qa(&mut a.conn, &key_a, "e2ee login-store A timeline").await?;
        let initial_b =
            subscribe_timeline_for_qa(&mut b.conn, &key_b, "e2ee login-store B timeline").await?;
        assert_no_decryption_failure_items(&initial_a, "e2ee login-store A initial")?;
        assert_no_decryption_failure_items(&initial_b, "e2ee login-store B initial")?;

        let phase_deadline = tokio::time::Instant::now() + E2EE_EVENT_TIMEOUT;
        send_after_rotation(
            &mut a.conn,
            &a_account_key,
            &room_id,
            "QA E2EE login-store rotation baseline",
            "qa-login-store-rotation-baseline",
            phase_deadline,
            "e2ee login-store rotation baseline",
        )
        .await?;
        wait_for_item_with_body_or_decryption_failure(
            &mut b.conn,
            &key_b,
            "QA E2EE login-store rotation baseline",
            "e2ee login-store rotation baseline receive",
        )
        .await?;
        let sessions_before = assert_inbound_sessions_start_at_zero(
            &b.conn,
            &room_id,
            "e2ee login-store rotation baseline",
        )
        .await?;
        force_rotate_outbound_session(&mut a.conn, &room_id, phase_deadline).await?;
        send_after_rotation(
            &mut a.conn,
            &a_account_key,
            &room_id,
            "QA E2EE login-store forced rotation",
            "qa-login-store-forced-rotation",
            phase_deadline,
            "e2ee login-store forced rotation",
        )
        .await?;
        wait_for_item_with_body_or_decryption_failure(
            &mut b.conn,
            &key_b,
            "QA E2EE login-store forced rotation",
            "e2ee login-store forced rotation receive",
        )
        .await?;
        let sessions_after = assert_inbound_sessions_start_at_zero(
            &b.conn,
            &room_id,
            "e2ee login-store forced rotation",
        )
        .await?;
        if sessions_after <= sessions_before {
            return Err(
                "e2ee login-store forced rotation did not create a new inbound session".to_owned(),
            );
        }
        println!("e2ee_login_store_forced_rotation_index0=ok");

        let phase_deadline = tokio::time::Instant::now() + E2EE_EVENT_TIMEOUT;
        let stopped = stop_qa_participant_for_offline(
            owner_b.take().ok_or("e2ee login-store B owner missing")?,
            data_dir_b.clone(),
            b_account_key.clone(),
            b_device_id.clone(),
        )
        .await?;
        stopped_b = Some(stopped.clone());
        send_after_rotation(
            &mut a.conn,
            &a_account_key,
            &room_id,
            "QA E2EE login-store fresh offline",
            "qa-login-store-fresh-offline",
            phase_deadline,
            "e2ee login-store fresh offline",
        )
        .await?;
        restart_recipient(
            &mut owner_b,
            &mut stopped_b,
            "e2ee login-store B fresh restart",
            phase_deadline,
        )
        .await?;
        let b = owner_b.as_mut().ok_or("e2ee login-store B owner missing")?;
        let b_key = TimelineKey::room(b_account_key.clone(), room_id.clone());
        let initial =
            subscribe_timeline_for_qa(&mut b.conn, &b_key, "e2ee login-store fresh receive")
                .await?;
        if find_timeline_item_with_body(&initial, "QA E2EE login-store fresh offline").is_none() {
            wait_for_item_with_body_or_decryption_failure(
                &mut b.conn,
                &b_key,
                "QA E2EE login-store fresh offline",
                "e2ee login-store fresh receive",
            )
            .await?;
        }
        assert_inbound_sessions_start_at_zero(&b.conn, &room_id, "e2ee login-store fresh receive")
            .await?;
        println!("e2ee_login_store_fresh_offline_index0=ok");

        let stopped_primary = stop_qa_participant_for_offline(
            owner_a.take().ok_or("e2ee login-store A owner missing")?,
            data_dir_a.clone(),
            a_account_key.clone(),
            a_device_id.clone(),
        )
        .await?;
        stopped_a = Some(stopped_primary.clone());
        owner_a = Some(
            restart_stopped_qa_participant(stopped_primary, "e2ee login-store A restore").await?,
        );
        stopped_a = None;
        let phase_deadline = tokio::time::Instant::now() + E2EE_EVENT_TIMEOUT;
        let stopped = stop_qa_participant_for_offline(
            owner_b.take().ok_or("e2ee login-store B owner missing")?,
            data_dir_b.clone(),
            b_account_key.clone(),
            b_device_id.clone(),
        )
        .await?;
        stopped_b = Some(stopped.clone());
        let a = owner_a.as_mut().ok_or("e2ee login-store A owner missing")?;
        let key_a = TimelineKey::room(a_account_key.clone(), room_id.clone());
        subscribe_timeline_for_qa(&mut a.conn, &key_a, "e2ee login-store restore sender").await?;
        send_after_rotation(
            &mut a.conn,
            &a_account_key,
            &room_id,
            "QA E2EE login-store restore offline",
            "qa-login-store-restore-offline",
            phase_deadline,
            "e2ee login-store restore offline",
        )
        .await?;
        let b = restart_stopped_qa_participant(stopped, "e2ee login-store B restore").await?;
        owner_b = Some(b);
        stopped_b = None;
        let b = owner_b.as_mut().ok_or("e2ee login-store B owner missing")?;
        let key_b = TimelineKey::room(b_account_key.clone(), room_id.clone());
        let initial =
            subscribe_timeline_for_qa(&mut b.conn, &key_b, "e2ee login-store restore receive")
                .await?;
        if find_timeline_item_with_body(&initial, "QA E2EE login-store restore offline").is_none() {
            wait_for_item_with_body_or_decryption_failure(
                &mut b.conn,
                &key_b,
                "QA E2EE login-store restore offline",
                "e2ee login-store restore receive",
            )
            .await?;
        }
        assert_inbound_sessions_start_at_zero(
            &b.conn,
            &room_id,
            "e2ee login-store restore receive",
        )
        .await?;
        println!("e2ee_login_store_restore_offline_index0=ok");

        // A second stop/restart keeps the restore path distinct from fresh login.
        let phase_deadline = tokio::time::Instant::now() + E2EE_EVENT_TIMEOUT;
        let stopped = stop_qa_participant_for_offline(
            owner_b.take().ok_or("e2ee login-store B owner missing")?,
            data_dir_b.clone(),
            b_account_key.clone(),
            b_device_id.clone(),
        )
        .await?;
        stopped_b = Some(stopped.clone());
        let a = owner_a.as_mut().ok_or("e2ee login-store A owner missing")?;
        let key_a = TimelineKey::room(a_account_key.clone(), room_id.clone());
        subscribe_timeline_for_qa(&mut a.conn, &key_a, "e2ee login-store restart sender").await?;
        send_after_rotation(
            &mut a.conn,
            &a_account_key,
            &room_id,
            "QA E2EE login-store restart offline",
            "qa-login-store-restart-offline",
            phase_deadline,
            "e2ee login-store restart offline",
        )
        .await?;
        owner_b =
            Some(restart_stopped_qa_participant(stopped, "e2ee login-store B restart").await?);
        stopped_b = None;
        let b = owner_b.as_mut().ok_or("e2ee login-store B owner missing")?;
        let key_b = TimelineKey::room(b_account_key.clone(), room_id.clone());
        let initial =
            subscribe_timeline_for_qa(&mut b.conn, &key_b, "e2ee login-store restart receive")
                .await?;
        if find_timeline_item_with_body(&initial, "QA E2EE login-store restart offline").is_none() {
            wait_for_item_with_body_or_decryption_failure(
                &mut b.conn,
                &key_b,
                "QA E2EE login-store restart offline",
                "e2ee login-store restart receive",
            )
            .await?;
        }
        assert_inbound_sessions_start_at_zero(
            &b.conn,
            &room_id,
            "e2ee login-store restart receive",
        )
        .await?;
        println!("e2ee_login_store_restart_offline_index0=ok");

        let phase_deadline = tokio::time::Instant::now() + E2EE_EVENT_TIMEOUT;
        {
            let a = owner_a.as_mut().ok_or("e2ee login-store A owner missing")?;
            a.runtime
                .inject_actions(vec![
                    koushi_state::AppAction::SessionAuthenticationInvalidated { soft_logout: true },
                ])
                .await;
            wait_for_locked_snapshot(&mut a.conn, "e2ee login-store injected soft logout").await?;
        }
        let reauth_id = {
            let a = owner_a.as_mut().ok_or("e2ee login-store A owner missing")?;
            let request_id = a.conn.next_request_id();
            tokio::time::timeout_at(
                phase_deadline,
                a.conn
                    .command(CoreCommand::Account(AccountCommand::SoftLogoutReauth {
                        request_id,
                        password: AuthSecret::new(config.password_a.clone()),
                    })),
            )
            .await
            .map_err(|_| "e2ee login-store reauth submit timed out".to_owned())?
            .map_err(|_| "e2ee login-store reauth submit failed".to_owned())?;
            request_id
        };
        {
            let a = owner_a.as_mut().ok_or("e2ee login-store A owner missing")?;
            let restored = tokio::time::timeout_at(
                phase_deadline,
                wait_for_logged_in(&mut a.conn, reauth_id, "e2ee login-store reauth"),
            )
            .await
            .map_err(|_| "e2ee login-store reauth timed out".to_owned())??;
            if restored != a_account_key
                || authenticated_session_info(&mut a.conn, "e2ee login-store reauth identity")?
                    .device_id
                    != a_device_id
            {
                return Err("e2ee login-store reauth changed identity".to_owned());
            }
            wait_for_ready_snapshot(&mut a.conn, "e2ee login-store reauth Ready").await?;
            start_sync_for_qa(&mut a.conn, "e2ee login-store reauth sync").await?;
        }
        let stopped = stop_qa_participant_for_offline(
            owner_b.take().ok_or("e2ee login-store B owner missing")?,
            data_dir_b.clone(),
            b_account_key.clone(),
            b_device_id.clone(),
        )
        .await?;
        stopped_b = Some(stopped.clone());
        let a = owner_a.as_mut().ok_or("e2ee login-store A owner missing")?;
        let key_a = TimelineKey::room(a_account_key.clone(), room_id.clone());
        subscribe_timeline_for_qa(&mut a.conn, &key_a, "e2ee login-store reauth sender").await?;
        send_after_rotation(
            &mut a.conn,
            &a_account_key,
            &room_id,
            "QA E2EE login-store reauth offline",
            "qa-login-store-reauth-offline",
            phase_deadline,
            "e2ee login-store reauth offline",
        )
        .await?;
        owner_b = Some(restart_stopped_qa_participant(stopped, "e2ee login-store B reauth").await?);
        stopped_b = None;
        let b = owner_b.as_mut().ok_or("e2ee login-store B owner missing")?;
        let key_b = TimelineKey::room(b_account_key.clone(), room_id.clone());
        let initial =
            subscribe_timeline_for_qa(&mut b.conn, &key_b, "e2ee login-store reauth receive")
                .await?;
        if find_timeline_item_with_body(&initial, "QA E2EE login-store reauth offline").is_none() {
            wait_for_item_with_body_or_decryption_failure(
                &mut b.conn,
                &key_b,
                "QA E2EE login-store reauth offline",
                "e2ee login-store reauth receive",
            )
            .await?;
        }
        assert_inbound_sessions_start_at_zero(&b.conn, &room_id, "e2ee login-store reauth receive")
            .await?;
        println!("e2ee_login_store_reauth_offline_index0=ok");

        let phase_deadline = tokio::time::Instant::now() + E2EE_EVENT_TIMEOUT;
        let a = owner_a.as_mut().ok_or("e2ee login-store A owner missing")?;
        let b = owner_b.as_mut().ok_or("e2ee login-store B owner missing")?;
        let key_a = TimelineKey::room(a_account_key.clone(), room_id.clone());
        subscribe_timeline_for_qa(&mut a.conn, &key_a, "e2ee login-store online sender").await?;
        send_after_rotation(
            &mut a.conn,
            &a_account_key,
            &room_id,
            "QA E2EE login-store online",
            "qa-login-store-online",
            phase_deadline,
            "e2ee login-store online",
        )
        .await?;
        let key_b = TimelineKey::room(b_account_key.clone(), room_id.clone());
        wait_for_item_with_body_or_decryption_failure(
            &mut b.conn,
            &key_b,
            "QA E2EE login-store online",
            "e2ee login-store online receive",
        )
        .await?;
        assert_inbound_sessions_start_at_zero(&b.conn, &room_id, "e2ee login-store online receive")
            .await?;
        println!("e2ee_login_store_online_index0=ok");

        let user_c = config
            .user_c
            .as_deref()
            .ok_or("e2ee login-store requires synthetic user C")?;
        let suffix = user_c
            .strip_prefix("qa_c_")
            .ok_or("e2ee login-store requires the local QA user naming contract")?;
        let password_c = std::env::var("KOUSHI_LOCAL_QA_PASSWORD_C")
            .unwrap_or_else(|_| format!("koushi-desktop-local-c-{suffix}"));
        let data_dir_c = qa_data_dir("e2ee-login-store-c");
        let mut c_outcome = login_synced_participant_for_qa(
            &config.homeserver,
            data_dir_c,
            user_c,
            &password_c,
            "Koushi Core QA C",
            "e2ee login-store C",
            "e2ee login-store bootstrap C",
            QaParticipantLoginGate::BootstrapNewIdentity,
        )
        .await?;
        let c_account_key = c_outcome.account_key.clone();
        let c_info = authenticated_session_info(&mut c_outcome.conn, "e2ee login-store C session")?;
        let c_device_id = c_info.device_id.clone();
        owner_c = Some(QaOwnedRuntimeParticipant::from(c_outcome));
        let room_id = create_room_for_qa(
            &mut a.conn,
            "QA E2EE login store group",
            true,
            "e2ee login-store create group",
        )
        .await?;
        rooms.push(room_id.clone());
        invite_user_for_qa(
            &mut a.conn,
            &room_id,
            &user_b_id,
            "e2ee login-store group invite B",
        )
        .await?;
        let user_c_id = format!("@{}:{}", user_c, config.server_name);
        invite_user_for_qa(
            &mut a.conn,
            &room_id,
            &user_c_id,
            "e2ee login-store group invite C",
        )
        .await?;
        wait_for_invite_in_snapshot(
            &mut b.conn,
            &room_id,
            Some(false),
            "e2ee login-store group B invite",
        )
        .await?;
        let c = owner_c.as_mut().ok_or("e2ee login-store C owner missing")?;
        wait_for_invite_in_snapshot(
            &mut c.conn,
            &room_id,
            Some(false),
            "e2ee login-store group C invite",
        )
        .await?;
        accept_invite_for_qa(&mut b.conn, &room_id, "e2ee login-store group B accept").await?;
        accept_invite_for_qa(&mut c.conn, &room_id, "e2ee login-store group C accept").await?;
        wait_for_room_in_room_list(&mut a.conn, &room_id, "e2ee login-store group A room").await?;
        wait_for_room_in_room_list(&mut b.conn, &room_id, "e2ee login-store group B room").await?;
        wait_for_room_in_room_list(&mut c.conn, &room_id, "e2ee login-store group C room").await?;
        let b_session =
            authenticated_session_info(&mut b.conn, "e2ee login-store group B identity")?;
        let c_session =
            authenticated_session_info(&mut c.conn, "e2ee login-store group C identity")?;
        refresh_device_keys_and_assert_known_for_qa(
            &mut a.conn,
            VerificationTarget {
                user_id: b_session.user_id.clone(),
                device_id: b_session.device_id.clone(),
            },
            "e2ee login-store group B key refresh",
        )
        .await?;
        refresh_device_keys_and_assert_known_for_qa(
            &mut a.conn,
            VerificationTarget {
                user_id: c_session.user_id.clone(),
                device_id: c_session.device_id.clone(),
            },
            "e2ee login-store group C key refresh",
        )
        .await?;
        let key_a = TimelineKey::room(a_account_key.clone(), room_id.clone());
        let key_b = TimelineKey::room(b_account_key.clone(), room_id.clone());
        let key_c = TimelineKey::room(c_account_key.clone(), room_id.clone());
        subscribe_timeline_for_qa(&mut a.conn, &key_a, "e2ee login-store group A timeline").await?;
        subscribe_timeline_for_qa(&mut b.conn, &key_b, "e2ee login-store group B timeline").await?;
        subscribe_timeline_for_qa(&mut c.conn, &key_c, "e2ee login-store group C timeline").await?;
        let phase_deadline = tokio::time::Instant::now() + E2EE_EVENT_TIMEOUT;
        send_after_rotation(
            &mut a.conn,
            &a_account_key,
            &room_id,
            "QA E2EE login-store group",
            "qa-login-store-group",
            phase_deadline,
            "e2ee login-store group",
        )
        .await?;
        wait_for_item_with_body_or_decryption_failure(
            &mut b.conn,
            &key_b,
            "QA E2EE login-store group",
            "e2ee login-store group B receive",
        )
        .await?;
        wait_for_item_with_body_or_decryption_failure(
            &mut c.conn,
            &key_c,
            "QA E2EE login-store group",
            "e2ee login-store group C receive",
        )
        .await?;
        assert_inbound_sessions_start_at_zero(
            &b.conn,
            &room_id,
            "e2ee login-store group B receive",
        )
        .await?;
        assert_inbound_sessions_start_at_zero(
            &c.conn,
            &room_id,
            "e2ee login-store group C receive",
        )
        .await?;
        println!("e2ee_login_store_group_index0=ok");

        let a_final = authenticated_session_info(&mut a.conn, "e2ee login-store final A")?;
        let b_final = authenticated_session_info(&mut b.conn, "e2ee login-store final B")?;
        let c = owner_c.as_mut().ok_or("e2ee login-store C owner missing")?;
        let c_final = authenticated_session_info(&mut c.conn, "e2ee login-store final C")?;
        if a_final.user_id != a_account_key.0
            || a_final.device_id != a_device_id
            || b_final.user_id != b_account_key.0
            || b_final.device_id != b_device_id
            || c_final.device_id != c_device_id
        {
            return Err("e2ee login-store identity continuity assertion failed".to_owned());
        }
        println!("e2ee_login_store_identity_stable=ok");
        Ok(())
    }
    .await;

    // A stopped recipient still owns a saved device. Reopen it before cleanup so
    // the same ordered logout guard can remove every owned session.
    if owner_b.is_none() {
        if let Some(stopped) = stopped_b.clone() {
            if let Ok(participant) =
                restart_stopped_qa_participant(stopped, "e2ee login-store cleanup B").await
            {
                owner_b = Some(participant);
                stopped_b = None;
            }
        }
    }
    if owner_a.is_none() {
        if let Some(stopped) = stopped_a.clone() {
            if let Ok(participant) =
                restart_stopped_qa_participant(stopped, "e2ee login-store cleanup A").await
            {
                owner_a = Some(participant);
                stopped_a = None;
            }
        }
    }

    let mut cleanup_failures = Vec::new();
    for (room_index, room_id) in rooms.iter().enumerate() {
        for (role_index, participant) in [&mut owner_a, &mut owner_b, &mut owner_c]
            .into_iter()
            .enumerate()
        {
            if room_index == 0 && role_index == 2 {
                continue;
            }
            if let Some(participant) = participant.as_mut()
                && leave_e2ee_login_store_room(
                    &mut participant.conn,
                    room_id,
                    "e2ee login-store room cleanup",
                )
                .await
                .is_err()
            {
                cleanup_failures.push(match (room_index, role_index) {
                    (0, 0) => "room1_a",
                    (0, 1) => "room1_b",
                    (0, 2) => "room1_c",
                    (1, 0) => "room2_a",
                    (1, 1) => "room2_b",
                    (1, 2) => "room2_c",
                    _ => "room_other",
                });
            }
        }
    }
    for (participant, label) in [
        (&mut owner_c, "e2ee login-store cleanup C"),
        (&mut owner_b, "e2ee login-store cleanup B"),
        (&mut owner_a, "e2ee login-store cleanup A"),
    ] {
        if let Some(participant) = participant.take() {
            if cleanup_owned_e2ee_participant_best_effort(participant, label)
                .await
                .is_err()
            {
                cleanup_failures.push(label);
            }
        }
    }
    if let Err(error) = stage_result {
        return Err(if !cleanup_failures.is_empty() {
            format!(
                "{error}; cleanup also failed ({})",
                cleanup_failures.join(",")
            )
        } else {
            error
        });
    }
    if !cleanup_failures.is_empty() {
        return Err(format!(
            "e2ee login-store cleanup failed ({})",
            cleanup_failures.join(",")
        ));
    }
    println!("e2ee_login_store=ok");
    Ok(())
}

pub(super) async fn run_gate_restore_stage(
    mut conn: CoreConnection,
    runtime: CoreRuntime,
    data_dir: std::path::PathBuf,
    account_key: AccountKey,
) -> Result<(), String> {
    println!("gate_restore_bootstrapped=ok");
    let stop_id = conn.next_request_id();
    tokio::time::timeout(
        EVENT_TIMEOUT,
        conn.command(CoreCommand::Sync(SyncCommand::Stop {
            request_id: stop_id,
        })),
    )
    .await
    .map_err(|_| "gate restore sync-stop submit timed out".to_owned())?
    .map_err(|error| format!("gate restore sync-stop submit: {error}"))?;
    wait_for_sync_stopped(&mut conn, stop_id, "gate restore sync stop").await?;
    drop(conn);
    tokio::time::timeout(EVENT_TIMEOUT, runtime.shutdown())
        .await
        .map_err(|_| "gate restore runtime shutdown timed out".to_owned())?;
    println!("gate_restore_shutdown_complete=ok");

    let reopened = CoreRuntime::start_with_data_dir(data_dir);
    let mut conn = reopened.attach();
    println!("gate_restore_runtime_spawned=ok");
    let query_id = conn.next_request_id();
    tokio::time::timeout(
        EVENT_TIMEOUT,
        conn.command(CoreCommand::Account(AccountCommand::QuerySavedSessions {
            request_id: query_id,
        })),
    )
    .await
    .map_err(|_| "gate restore query submit timed out".to_owned())?
    .map_err(|error| format!("gate restore query submit: {error}"))?;
    println!("gate_restore_query_sent=ok");
    wait_for_saved_session_presence(&mut conn, query_id, &account_key).await?;
    println!("gate_restore_query_result=ok");

    let restore_started_at = std::time::Instant::now();
    let restore_id = conn.next_request_id();
    tokio::time::timeout(
        EVENT_TIMEOUT,
        conn.command(CoreCommand::Account(AccountCommand::RestoreSession {
            request_id: restore_id,
            account_key: account_key.clone(),
        })),
    )
    .await
    .map_err(|_| "gate restore restore submit timed out".to_owned())?
    .map_err(|error| format!("gate restore restore submit: {error}"))?;
    println!("gate_restore_restore_sent=ok");
    wait_for_session_restored(&mut conn, restore_id, &account_key, "gate restore").await?;
    println!("gate_restore_restore_result=ok");
    wait_for_ready_snapshot(&mut conn, "gate restore Ready").await?;
    if restore_started_at.elapsed() > GATE_RESTORE_READY_BUDGET {
        return Err("gate restore exceeded bounded Ready budget".to_owned());
    }
    println!("gate_restore_ready=ok");
    println!("gate_verified_restore=ok");
    drop(conn);
    tokio::time::timeout(EVENT_TIMEOUT, reopened.shutdown())
        .await
        .map_err(|_| "gate restore reopened shutdown timed out".to_owned())?;
    Ok(())
}

async fn delete_qa_current_device(
    session: &koushi_sdk::MatrixClientSession,
    password: &str,
) -> Result<(), String> {
    let device_ids = [matrix_sdk::ruma::OwnedDeviceId::from(
        session.info.device_id.as_str(),
    )];
    let uiaa_session = match session.client().delete_devices(&device_ids, None).await {
        Ok(_) => return Ok(()),
        Err(error) => error
            .as_uiaa_response()
            .and_then(|uiaa| uiaa.session.clone())
            .ok_or_else(|| "no-proof initial device delete failed".to_owned())?,
    };
    let identifier = matrix_sdk::ruma::api::client::uiaa::UserIdentifier::Matrix(
        matrix_sdk::ruma::api::client::uiaa::MatrixUserIdentifier::new(
            session.info.user_id.clone(),
        ),
    );
    let mut password_auth =
        matrix_sdk::ruma::api::client::uiaa::Password::new(identifier, password.to_owned());
    password_auth.session = Some(uiaa_session);
    session
        .client()
        .delete_devices(
            &device_ids,
            Some(matrix_sdk::ruma::api::client::uiaa::AuthData::Password(
                password_auth,
            )),
        )
        .await
        .map(|_| ())
        .map_err(|_| "no-proof authenticated device delete failed".to_owned())
}

pub(super) async fn run_gate_no_proof_stage(config: &QaConfig) -> Result<(), String> {
    let raw = koushi_sdk::login_with_password(&koushi_state::LoginRequest {
        homeserver: config.homeserver.clone(),
        username: config.user_a.clone(),
        password: AuthSecret::new(config.password_a.clone()),
        device_display_name: Some("Koushi No Proof Fixture".to_owned()),
    })
    .await
    .map_err(|_| "no-proof fixture login failed".to_owned())?;
    koushi_sdk::sync_once(&raw)
        .await
        .map_err(|_| "no-proof fixture sync failed".to_owned())?;
    koushi_sdk::bootstrap_cross_signing(&raw, Some(&AuthSecret::new(config.password_a.clone())))
        .await
        .map_err(|_| "no-proof cross-signing bootstrap failed".to_owned())?;
    delete_qa_current_device(&raw, &config.password_a).await?;
    let _ = koushi_sdk::close_session_stores(&raw).await;
    drop(raw);

    let data_dir = qa_data_dir("gate-no-proof");
    let runtime = CoreRuntime::start_with_data_dir(data_dir.clone());
    let mut conn = runtime.attach();
    let login_id = conn.next_request_id();
    conn.command(CoreCommand::Account(AccountCommand::LoginPassword {
        request_id: login_id,
        request: koushi_state::LoginRequest {
            homeserver: config.homeserver.clone(),
            username: config.user_a.clone(),
            password: AuthSecret::new(config.password_a.clone()),
            device_display_name: Some("Koushi No Proof Core".to_owned()),
        },
        platform: koushi_state::DisplayPlatform::Linux,
    }))
    .await
    .map_err(|_| "no-proof Core login submit failed".to_owned())?;
    let deadline = tokio::time::Instant::now() + E2EE_EVENT_TIMEOUT;
    let mut saw_rejecting = false;
    loop {
        saw_rejecting |= matches!(conn.snapshot().session, SessionState::Rejecting { .. });
        if matches!(conn.snapshot().session, SessionState::SignedOut) && saw_rejecting {
            break;
        }
        tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| "no-proof rejection timed out".to_owned())?
            .map_err(|_| "no-proof event stream closed".to_owned())?;
    }
    println!("gate_no_proof_rejected=ok");
    drop(conn);
    runtime.shutdown().await;

    let reopened = CoreRuntime::start_with_data_dir(data_dir);
    let mut reopened_conn = reopened.attach();
    let restore_id = reopened_conn.next_request_id();
    reopened_conn
        .command(CoreCommand::Account(AccountCommand::RestoreLastSession {
            request_id: restore_id,
        }))
        .await
        .map_err(|_| "no-proof restart restore submit failed".to_owned())?;
    let failure = wait_for_operation_failed_and_signed_out(
        &mut reopened_conn,
        restore_id,
        "no-proof restart restore",
    )
    .await?;
    if failure != CoreFailure::SessionNotFound {
        return Err("no-proof restart did not remain SignedOut".to_owned());
    }
    println!("gate_no_proof_restart_signed_out=ok");
    drop(reopened_conn);
    reopened.shutdown().await;
    Ok(())
}

pub(super) async fn run_gate_negative_stage(
    config: &QaConfig,
    conn_a: &mut CoreConnection,
    recovery_secret: &AuthSecret,
) -> Result<(), String> {
    let session_a = authenticated_session_info(conn_a, "gate negative primary session")?;
    let runtime_a2 = start_isolated_qa_runtime("gate-negative-a2")?;
    let mut conn_a2 = runtime_a2.attach();
    let login_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Account(AccountCommand::LoginPassword {
            request_id: login_id,
            request: koushi_state::LoginRequest {
                homeserver: config.homeserver.clone(),
                username: config.user_a.clone(),
                password: AuthSecret::new(config.password_a.clone()),
                device_display_name: Some("Koushi Gate Negative A2".to_owned()),
            },
            platform: koushi_state::DisplayPlatform::Linux,
        }))
        .await
        .map_err(|error| format!("gate negative login submit: {error}"))?;
    let session_a2 = wait_for_existing_identity_gate(&mut conn_a2, "gate negative A2").await?;
    verify_provisional_second_device_for_qa(
        conn_a,
        &mut conn_a2,
        &session_a,
        &session_a2,
        "gate negative mismatch",
        SasQaOutcome::Mismatch,
    )
    .await?;
    println!("gate_sas_mismatch_retryable=ok");
    let retry_session =
        wait_for_existing_identity_gate(&mut conn_a2, "gate negative retry").await?;
    verify_provisional_second_device_for_qa(
        conn_a,
        &mut conn_a2,
        &session_a,
        &retry_session,
        "gate negative retry success",
        SasQaOutcome::Success,
    )
    .await?;
    let _ = wait_for_logged_in(&mut conn_a2, login_id, "gate negative A2 login").await?;
    wait_for_ready_snapshot(&mut conn_a2, "gate negative A2 Ready").await?;
    println!("gate_sas_retry_ready=ok");
    drop(conn_a2);
    runtime_a2.shutdown().await;

    let runtime_a3 = start_isolated_qa_runtime("gate-negative-a3")?;
    let mut conn_a3 = runtime_a3.attach();
    let login_a3 = conn_a3.next_request_id();
    conn_a3
        .command(CoreCommand::Account(AccountCommand::LoginPassword {
            request_id: login_a3,
            request: koushi_state::LoginRequest {
                homeserver: config.homeserver.clone(),
                username: config.user_a.clone(),
                password: AuthSecret::new(config.password_a.clone()),
                device_display_name: Some("Koushi Gate Negative A3".to_owned()),
            },
            platform: koushi_state::DisplayPlatform::Linux,
        }))
        .await
        .map_err(|error| format!("gate negative A3 login submit: {error}"))?;
    let session_a3 = wait_for_existing_identity_gate(&mut conn_a3, "gate negative A3").await?;
    verify_provisional_second_device_for_qa(
        conn_a,
        &mut conn_a3,
        &session_a,
        &session_a3,
        "gate negative user cancel",
        SasQaOutcome::UserCancel,
    )
    .await?;
    println!("gate_sas_user_cancel_retryable=ok");
    let retry_a3 = wait_for_existing_identity_gate(&mut conn_a3, "gate negative A3 retry").await?;
    verify_provisional_second_device_for_qa(
        conn_a,
        &mut conn_a3,
        &session_a,
        &retry_a3,
        "gate negative user-cancel retry success",
        SasQaOutcome::Success,
    )
    .await?;
    let _ = wait_for_logged_in(&mut conn_a3, login_a3, "gate negative A3 login").await?;
    wait_for_ready_snapshot(&mut conn_a3, "gate negative A3 Ready").await?;
    println!("gate_sas_user_cancel_retry_ready=ok");
    drop(conn_a3);
    runtime_a3.shutdown().await;

    let runtime_a4 = start_isolated_qa_runtime("gate-negative-a4")?;
    let mut conn_a4 = runtime_a4.attach();
    let login_a4 = conn_a4.next_request_id();
    conn_a4
        .command(CoreCommand::Account(AccountCommand::LoginPassword {
            request_id: login_a4,
            request: koushi_state::LoginRequest {
                homeserver: config.homeserver.clone(),
                username: config.user_a.clone(),
                password: AuthSecret::new(config.password_a.clone()),
                device_display_name: Some("Koushi Gate Negative A4".to_owned()),
            },
            platform: koushi_state::DisplayPlatform::Linux,
        }))
        .await
        .map_err(|error| format!("gate negative A4 login submit: {error}"))?;
    let session_a4 = wait_for_existing_identity_gate(&mut conn_a4, "gate negative A4").await?;
    verify_provisional_second_device_for_qa(
        conn_a,
        &mut conn_a4,
        &session_a,
        &session_a4,
        "gate negative timeout",
        SasQaOutcome::Timeout,
    )
    .await?;
    println!("gate_sas_timeout_retryable=ok");
    let retry_a4 = wait_for_existing_identity_gate(&mut conn_a4, "gate negative A4 retry").await?;
    verify_provisional_second_device_for_qa(
        conn_a,
        &mut conn_a4,
        &session_a,
        &retry_a4,
        "gate negative timeout retry success",
        SasQaOutcome::Success,
    )
    .await?;
    let _ = wait_for_logged_in(&mut conn_a4, login_a4, "gate negative A4 login").await?;
    wait_for_ready_snapshot(&mut conn_a4, "gate negative A4 Ready").await?;
    println!("gate_sas_timeout_retry_ready=ok");
    drop(conn_a4);
    runtime_a4.shutdown().await;

    let runtime_a5 = start_isolated_qa_runtime("gate-negative-a5")?;
    let mut conn_a5 = runtime_a5.attach();
    let login_a5 = conn_a5.next_request_id();
    conn_a5
        .command(CoreCommand::Account(AccountCommand::LoginPassword {
            request_id: login_a5,
            request: koushi_state::LoginRequest {
                homeserver: config.homeserver.clone(),
                username: config.user_a.clone(),
                password: AuthSecret::new(config.password_a.clone()),
                device_display_name: Some("Koushi Gate Negative A5".to_owned()),
            },
            platform: koushi_state::DisplayPlatform::Linux,
        }))
        .await
        .map_err(|error| format!("gate negative A5 login submit: {error}"))?;
    wait_for_recovery_gate(&mut conn_a5, "gate negative A5").await?;
    let invalid_recovery = conn_a5.next_request_id();
    conn_a5
        .command(CoreCommand::Account(AccountCommand::SubmitRecovery {
            request_id: invalid_recovery,
            request: RecoveryRequest {
                secret: AuthSecret::new(QA_WRONG_RECOVERY_SECRET.to_owned()),
            },
        }))
        .await
        .map_err(|error| format!("gate negative invalid recovery submit: {error}"))?;
    let failure = wait_for_operation_failed(
        &mut conn_a5,
        invalid_recovery,
        "gate negative invalid recovery",
    )
    .await?;
    if !matches!(failure, CoreFailure::RecoveryFailed { .. }) {
        return Err("gate negative invalid recovery returned unexpected failure kind".to_owned());
    }
    wait_for_recovery_gate(&mut conn_a5, "gate negative A5 retry").await?;
    println!("gate_recovery_invalid_retryable=ok");
    let valid_recovery = conn_a5.next_request_id();
    conn_a5
        .command(CoreCommand::Account(AccountCommand::SubmitRecovery {
            request_id: valid_recovery,
            request: RecoveryRequest {
                secret: recovery_secret.clone(),
            },
        }))
        .await
        .map_err(|error| format!("gate negative valid recovery submit: {error}"))?;
    wait_for_ready_snapshot(&mut conn_a5, "gate negative recovery Ready").await?;
    println!("gate_recovery_retry_ready=ok");
    drop(conn_a5);
    runtime_a5.shutdown().await;

    let runtime_a6 = start_isolated_qa_runtime("gate-negative-a6")?;
    let mut conn_a6 = runtime_a6.attach();
    let login_a6 = conn_a6.next_request_id();
    conn_a6
        .command(CoreCommand::Account(AccountCommand::LoginPassword {
            request_id: login_a6,
            request: koushi_state::LoginRequest {
                homeserver: config.homeserver.clone(),
                username: config.user_a.clone(),
                password: AuthSecret::new(config.password_a.clone()),
                device_display_name: Some("Koushi Gate Negative A6".to_owned()),
            },
            platform: koushi_state::DisplayPlatform::Linux,
        }))
        .await
        .map_err(|error| format!("gate negative A6 login submit: {error}"))?;
    wait_for_recovery_gate(&mut conn_a6, "gate negative A6").await?;
    let cancelled_recovery = conn_a6.next_request_id();
    conn_a6
        .command(CoreCommand::Account(AccountCommand::SubmitRecovery {
            request_id: cancelled_recovery,
            request: RecoveryRequest {
                secret: recovery_secret.clone(),
            },
        }))
        .await
        .map_err(|error| format!("gate negative cancelled recovery submit: {error}"))?;
    wait_for_matching_recovery_flow(
        &mut conn_a6,
        cancelled_recovery.sequence,
        "gate negative cancelled recovery",
    )
    .await?;
    let cancel_recovery = conn_a6.next_request_id();
    conn_a6
        .command(CoreCommand::Account(AccountCommand::CancelVerification {
            request_id: cancel_recovery,
            flow_id: cancelled_recovery.sequence,
            reason: koushi_state::VerificationCancelReason::User,
        }))
        .await
        .map_err(|error| format!("gate negative recovery cancel submit: {error}"))?;
    wait_for_recovery_gate(&mut conn_a6, "gate negative A6 cancelled retry").await?;
    println!("gate_recovery_cancel_retryable=ok");
    let retry_recovery = conn_a6.next_request_id();
    conn_a6
        .command(CoreCommand::Account(AccountCommand::SubmitRecovery {
            request_id: retry_recovery,
            request: RecoveryRequest {
                secret: recovery_secret.clone(),
            },
        }))
        .await
        .map_err(|error| format!("gate negative recovery retry submit: {error}"))?;
    wait_for_ready_snapshot(&mut conn_a6, "gate negative cancelled recovery Ready").await?;
    println!("gate_recovery_cancel_retry_ready=ok");
    let account_key_a6 = AccountKey(
        authenticated_session_info(&mut conn_a6, "gate negative A6 reset session")?.user_id,
    );
    reset_identity_for_qa(
        &mut conn_a6,
        &account_key_a6,
        config.password_a.clone(),
        "gate negative trust loss reset",
    )
    .await?;
    wait_for_locked_snapshot(conn_a, "gate negative primary trust loss").await?;
    println!("gate_trust_loss_locked=ok");
    let blocked_sync = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Sync(SyncCommand::Start {
            request_id: blocked_sync,
        }))
        .await
        .map_err(|error| format!("gate negative locked sync submit: {error}"))?;
    let failure =
        wait_for_operation_failed(conn_a, blocked_sync, "gate negative locked normal command")
            .await?;
    if failure != CoreFailure::SessionRequired {
        return Err("gate negative locked command returned unexpected failure kind".to_owned());
    }
    println!("gate_trust_loss_commands_blocked=ok");
    drop(conn_a6);
    runtime_a6.shutdown().await;
    Ok(())
}

pub(super) async fn run_provisional_device_cleanup_qa(config: &QaConfig) -> Result<(), String> {
    let runtime = CoreRuntime::start_with_data_dir(qa_data_dir("gate-device-cleanup"));
    let mut conn = runtime.attach();
    let result = async {
        let removed_session =
            login_until_device_cleanup_offered(&mut conn, config, "device cleanup first login")
                .await?;
        drive_remote_first_device_cleanup(
            &mut conn,
            &config.password_a,
            "device cleanup first device",
        )
        .await?;

        let replacement_session =
            login_until_device_cleanup_offered(&mut conn, config, "device cleanup replacement")
                .await?;
        if replacement_session.device_id == removed_session.device_id {
            return Err(
                "device cleanup replacement login reused the removed server device".to_owned(),
            );
        }
        audit_removed_device_absent_from_server(
            config,
            &removed_session.device_id,
            &replacement_session.device_id,
        )
        .await?;
        println!("device_cleanup_remote_first=ok");
        println!("device_cleanup_relogin_new_device=ok");

        drive_remote_first_device_cleanup(
            &mut conn,
            &config.password_a,
            "device cleanup replacement device",
        )
        .await?;
        Ok(())
    }
    .await;

    drop(conn);
    runtime.shutdown().await;
    result
}

pub(super) async fn audit_removed_device_absent_from_server(
    config: &QaConfig,
    removed_device_id: &str,
    replacement_device_id: &str,
) -> Result<(), String> {
    let auditor = koushi_sdk::login_with_password(&koushi_state::LoginRequest {
        homeserver: config.homeserver.clone(),
        username: config.user_a.clone(),
        password: AuthSecret::new(config.password_a.clone()),
        device_display_name: Some("Koushi Device Cleanup Auditor".to_owned()),
    })
    .await
    .map_err(|_| "device cleanup audit login failed".to_owned())?;
    let devices = auditor
        .client()
        .devices()
        .await
        .map_err(|_| "device cleanup audit device-list request failed".to_owned());
    let cleanup = cleanup_qa_auditor_device(&auditor, &config.password_a).await;
    let _ = koushi_sdk::close_session_stores(&auditor).await;
    drop(auditor);

    let devices = devices?;
    cleanup?;
    if devices
        .devices
        .iter()
        .any(|device| device.device_id.as_str() == removed_device_id)
    {
        return Err("device cleanup audit found the removed device on the homeserver".to_owned());
    }
    if !devices
        .devices
        .iter()
        .any(|device| device.device_id.as_str() == replacement_device_id)
    {
        return Err(
            "device cleanup audit did not find the replacement device on the homeserver".to_owned(),
        );
    }
    Ok(())
}

async fn cleanup_qa_auditor_device(
    auditor: &koushi_sdk::MatrixClientSession,
    password: &str,
) -> Result<(), String> {
    let initial = koushi_sdk::cleanup_current_device(auditor, None, None)
        .await
        .map_err(|_| "device cleanup audit device removal failed".to_owned())?;
    match initial {
        koushi_sdk::MatrixDeviceCleanupOutcome::Settled(_) => Ok(()),
        koushi_sdk::MatrixDeviceCleanupOutcome::UiaaRequired { session } => {
            match koushi_sdk::cleanup_current_device(
                auditor,
                Some(&AuthSecret::new(password.to_owned())),
                session.as_deref(),
            )
            .await
            .map_err(|_| "device cleanup audit authenticated removal failed".to_owned())?
            {
                koushi_sdk::MatrixDeviceCleanupOutcome::Settled(_) => Ok(()),
                koushi_sdk::MatrixDeviceCleanupOutcome::UiaaRequired { .. } => {
                    Err("device cleanup audit repeated its authentication challenge".to_owned())
                }
            }
        }
    }
}

pub(super) async fn login_until_device_cleanup_offered(
    conn: &mut CoreConnection,
    config: &QaConfig,
    label: &str,
) -> Result<SessionInfo, String> {
    let login_id = conn.next_request_id();
    conn.command(CoreCommand::Account(AccountCommand::LoginPassword {
        request_id: login_id,
        request: koushi_state::LoginRequest {
            homeserver: config.homeserver.clone(),
            username: config.user_a.clone(),
            password: AuthSecret::new(config.password_a.clone()),
            device_display_name: Some("Koushi Device Cleanup QA".to_owned()),
        },
        platform: koushi_state::DisplayPlatform::Linux,
    }))
    .await
    .map_err(|error| format!("{label}: login submit failed: {error}"))?;
    wait_for_recovery_gate(conn, label).await?;
    let session = authenticated_session_info(conn, label)?;

    let invalid_recovery = conn.next_request_id();
    conn.command(CoreCommand::Account(AccountCommand::SubmitRecovery {
        request_id: invalid_recovery,
        request: RecoveryRequest {
            secret: AuthSecret::new(QA_WRONG_RECOVERY_SECRET.to_owned()),
        },
    }))
    .await
    .map_err(|error| format!("{label}: invalid recovery submit failed: {error}"))?;
    wait_for_device_cleanup_offered(conn, label).await?;
    Ok(session)
}

async fn wait_for_device_cleanup_offered(
    conn: &mut CoreConnection,
    label: &str,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + E2EE_EVENT_TIMEOUT;
    loop {
        if matches!(
            conn.snapshot().device_cleanup,
            DeviceCleanupState::Offered { .. }
        ) {
            return Ok(());
        }
        tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for device cleanup offer"))?
            .map_err(|_| format!("{label}: event stream closed"))?;
    }
}

async fn drive_remote_first_device_cleanup(
    conn: &mut CoreConnection,
    password: &str,
    label: &str,
) -> Result<(), String> {
    let start_request_id = conn.next_request_id();
    conn.command(CoreCommand::Account(AccountCommand::StartDeviceCleanup {
        request_id: start_request_id,
    }))
    .await
    .map_err(|error| format!("{label}: cleanup submit failed: {error}"))?;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(150);
    let mut submitted_uia_flow = None;
    let mut retried_local_reset = false;
    loop {
        let snapshot = conn.snapshot();
        if matches!(snapshot.session, SessionState::SignedOut) {
            return Ok(());
        }
        match snapshot.device_cleanup {
            DeviceCleanupState::AwaitingUia { flow_id, .. }
                if submitted_uia_flow != Some(flow_id) =>
            {
                submitted_uia_flow = Some(flow_id);
                let request_id = conn.next_request_id();
                conn.command(CoreCommand::Account(
                    AccountCommand::SubmitDeviceCleanupUia {
                        request_id,
                        flow_id,
                        password: AuthSecret::new(password.to_owned()),
                    },
                ))
                .await
                .map_err(|error| format!("{label}: cleanup UIAA submit failed: {error}"))?;
            }
            DeviceCleanupState::RemoteFailed {
                failure, auth_mode, ..
            } => {
                return Err(format!(
                    "{label}: remote cleanup failed; mode={auth_mode:?} failure={failure:?}"
                ));
            }
            DeviceCleanupState::LocalResetFailed {
                mode: DeviceCleanupLocalMode::RemoteRemoved { .. },
                ..
            } if !retried_local_reset => {
                retried_local_reset = true;
                let request_id = conn.next_request_id();
                conn.command(CoreCommand::Account(AccountCommand::StartDeviceCleanup {
                    request_id,
                }))
                .await
                .map_err(|error| format!("{label}: local cleanup retry failed: {error}"))?;
            }
            DeviceCleanupState::LocalResetFailed { .. } => {
                return Err(format!("{label}: local cleanup failed after retry"));
            }
            _ => {}
        }

        tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for cleanup completion"))?
            .map_err(|_| format!("{label}: event stream closed"))?;
    }
}

async fn wait_for_saved_session_presence(
    conn: &mut CoreConnection,
    request_id: RequestId,
    expected: &AccountKey,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    loop {
        let event = tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| "timed out waiting for saved-session readiness".to_owned())?
            .map_err(|lag| {
                format!(
                    "saved-session readiness event lagged (skipped={})",
                    lag.skipped
                )
            })?;
        match event {
            CoreEvent::Account(AccountEvent::SavedSessionsListed {
                request_id: event_id,
                sessions,
            }) if event_id == request_id => {
                if sessions.iter().any(|session| session.user_id == expected.0) {
                    return Ok(());
                }
                return Err(format!(
                    "saved-session readiness missing expected account; saved_count={}",
                    sessions.len()
                ));
            }
            CoreEvent::OperationFailed {
                request_id: event_id,
                failure,
            } if event_id == request_id => {
                return Err(format!("saved-session readiness failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

pub(super) async fn run_e2ee_trust_stage(
    config: &QaConfig,
    conn_a: &mut CoreConnection,
    account_key_a: &AccountKey,
    recipient_base: Option<(&mut CoreConnection, &AccountKey)>,
) -> Result<(), String> {
    let session_a = authenticated_session_info(conn_a, "session A info for E2EE trust")?;

    // The login gate already bootstrapped and authoritatively promoted this
    // session. Re-running bootstrap here would rotate the identity and
    // invalidate the proof device that A2 is about to use.
    println!("e2ee_cross_signing_reused=ok");
    println!("e2ee_cross_signing=ok");

    let key_backup_seed_room_id =
        seed_encrypted_room_key_for_qa(conn_a, account_key_a, "seed key backup room A").await?;
    println!("e2ee_key_backup_seed=ok");

    let key_backup_version = enable_key_backup_for_qa(
        conn_a,
        account_key_a,
        Some(AuthSecret::new(config.password_a.clone())),
        "enable key backup A",
    )
    .await?;
    println!("e2ee_key_backup_enable=ok");

    let runtime_a2 = start_isolated_qa_runtime("a2")?;
    let conn_a2 = runtime_a2.attach();
    let mut owned_a2 = QaOwnedRuntimeParticipant::new(runtime_a2, conn_a2);
    let a2_stage_result: Result<(), String> = async {
        let login_a2_id = owned_a2.conn.next_request_id();
        owned_a2
            .conn
            .command(CoreCommand::Account(AccountCommand::LoginPassword {
                request_id: login_a2_id,
                request: koushi_state::LoginRequest {
                    homeserver: config.homeserver.clone(),
                    username: config.user_a.clone(),
                    password: AuthSecret::new(config.password_a.clone()),
                    device_display_name: Some("Koushi Core QA A2".to_owned()),
                },
                platform: koushi_state::DisplayPlatform::Linux,
            }))
            .await
            .map_err(|e| format!("submit login A2: {e}"))?;
        owned_a2.mark_login_submitted();

        let session_a2 =
            wait_for_existing_identity_gate(&mut owned_a2.conn, "session A2 gate").await?;
        verify_provisional_second_device_for_qa(
            conn_a,
            &mut owned_a2.conn,
            &session_a,
            &session_a2,
            "e2ee gated self verification A/A2",
            SasQaOutcome::Success,
        )
        .await?;
        let account_key_a2 =
            wait_for_logged_in(&mut owned_a2.conn, login_a2_id, "login A2").await?;
        owned_a2.mark_logged_in(account_key_a2.clone());
        let conn_a2 = &mut owned_a2.conn;
        wait_for_ready_snapshot(conn_a2, "session A2 Ready").await?;
        println!("gate_own_sas=ok");

        let sync_start_a2_id = conn_a2.next_request_id();
        conn_a2
            .command(CoreCommand::Sync(SyncCommand::Start {
                request_id: sync_start_a2_id,
            }))
            .await
            .map_err(|e| format!("submit sync start A2: {e}"))?;
        wait_for_sync_started_and_running(conn_a2, sync_start_a2_id, "sync start A2").await?;

        wait_for_room_in_room_list(
            conn_a2,
            &key_backup_seed_room_id,
            "room list A2 after key backup seed",
        )
        .await?;

        restore_key_backup_failure_for_qa(
            conn_a2,
            &account_key_a2,
            Some(key_backup_version.clone()),
            "restore key backup failure A2",
        )
        .await?;
        println!("e2ee_key_backup_restore_failure=ok");

        restore_key_backup_success_for_qa(
            conn_a2,
            &account_key_a2,
            Some(key_backup_version),
            AuthSecret::new(config.password_a.clone()),
            "restore key backup success A2",
        )
        .await?;
        println!("joined_room_restore=ok");

        println!("e2ee_verification=ok");

        verify_second_device_room_key_delivery_for_qa(
            conn_a,
            conn_a2,
            account_key_a,
            &account_key_a2,
            &key_backup_seed_room_id,
        )
        .await?;
        println!("e2ee_second_device_decrypt=ok");

        verify_multi_user_multi_device_room_key_delivery_for_qa(
            config,
            conn_a,
            conn_a2,
            account_key_a,
            &account_key_a2,
            recipient_base,
        )
        .await?;
        println!("e2ee_multi_user_multi_device_decrypt=ok");
        Ok(())
    }
    .await;

    finish_e2ee_recipient_stage_with_owned_cleanup(
        a2_stage_result,
        Some(owned_a2),
        |participant| async move {
            cleanup_owned_e2ee_participant_best_effort(participant, "cleanup secondary device")
                .await
        },
    )
    .await?;

    if config.allow_identity_reset {
        reset_identity_for_qa(
            conn_a,
            account_key_a,
            config.password_a.clone(),
            "reset identity A",
        )
        .await?;
        println!("e2ee_identity_reset=ok");
    } else {
        println!("e2ee_identity_reset=skipped");
    }
    println!("e2ee_trust=ok");

    Ok(())
}

async fn wait_for_local_encryption_health(
    conn: &mut CoreConnection,
    request_id: RequestId,
    expected: LocalEncryptionHealth,
    label: &str,
) -> Result<(), String> {
    let expected_state = LocalEncryptionState::from(expected);
    if conn.snapshot().local_encryption == expected_state {
        return Ok(());
    }

    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for local encryption health"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::StateDelta(_) if conn.snapshot().local_encryption == expected_state => {
                return Ok(());
            }
            CoreEvent::LocalEncryption(LocalEncryptionEvent::HealthChanged { health })
                if health == expected && conn.snapshot().local_encryption == expected_state =>
            {
                return Ok(());
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!(
                    "{label}: local encryption health failed: {failure:?}"
                ));
            }
            _ if conn.snapshot().local_encryption == expected_state => {
                return Ok(());
            }
            _ => {}
        }
    }
}

async fn wait_for_native_attention_state(
    conn: &mut CoreConnection,
    request_id: RequestId,
    expected: &NativeAttentionState,
    label: &str,
) -> Result<(), String> {
    if conn.snapshot().native_attention == *expected {
        return Ok(());
    }

    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for native attention summary"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::StateDelta(_) if conn.snapshot().native_attention == *expected => {
                return Ok(());
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!(
                    "{label}: native attention update failed: {failure:?}"
                ));
            }
            _ if conn.snapshot().native_attention == *expected => {
                return Ok(());
            }
            _ => {}
        }
    }
}

pub(super) async fn run_session_status_stage(conn: &mut CoreConnection) -> Result<(), String> {
    let expected_device_id = match &conn.snapshot().session {
        SessionState::Ready(info) => info.device_id.clone(),
        _ => return Err("session_status: current session is not Ready".to_owned()),
    };
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Account(
        AccountCommand::RefreshCurrentSessionStatus {
            request_id,
            trigger: SessionStatusRefreshTrigger::Manual,
        },
    ))
    .await
    .map_err(|_| "session_status: refresh command was rejected".to_owned())?;

    let mut saw_checking = false;
    let deadline = QaEventDeadline::after(EVENT_TIMEOUT);
    loop {
        let event = deadline
            .recv(conn)
            .await
            .map_err(|_| "session_status: timed out waiting for settlement".to_owned())?
            .map_err(|lag| {
                format!(
                    "session_status: event stream lagged (skipped={})",
                    lag.skipped
                )
            })?;
        if !matches!(event, CoreEvent::StateDelta(_)) {
            continue;
        }
        let snapshot = conn.snapshot();
        match &snapshot.current_session_status {
            CurrentSessionStatusState::Checking {
                request_id: observed_request_id,
                trigger: SessionStatusRefreshTrigger::Manual,
                ..
            } if *observed_request_id == request_id.sequence => {
                if !saw_checking {
                    println!("session_status_checking=ok");
                    saw_checking = true;
                }
            }
            CurrentSessionStatusState::Ready {
                request_id: observed_request_id,
                details,
            } if *observed_request_id == request_id.sequence => {
                if !saw_checking {
                    return Err(
                        "session_status: Ready was observed without the Checking transition"
                            .to_owned(),
                    );
                }
                if details.device_id != expected_device_id {
                    return Err(
                        "session_status: SDK facts did not describe the current device".to_owned(),
                    );
                }
                if details.device_display_name.as_deref() != Some(DEVICE_A) {
                    return Err(
                        "session_status: server-side current-device name did not match login"
                            .to_owned(),
                    );
                }
                if details.authentication_method != SessionAuthenticationMethod::Password
                    || details.sync_state != CurrentSessionSyncState::Running
                {
                    return Err(
                        "session_status: authentication or sync facts did not match runtime"
                            .to_owned(),
                    );
                }
                println!("session_status_ready=ok");
                println!("session_status_device=ok");
                println!("session_status=ok");
                return Ok(());
            }
            CurrentSessionStatusState::Failed {
                request_id: observed_request_id,
                ..
            } if *observed_request_id == request_id.sequence => {
                return Err("session_status: refresh settled with a coarse failure".to_owned());
            }
            _ => {}
        }
    }
}

pub(super) async fn run_credential_health_stage(conn: &mut CoreConnection) -> Result<(), String> {
    let probe_id = conn.next_request_id();
    conn.command(CoreCommand::Account(
        AccountCommand::ProbeLocalEncryptionHealth {
            request_id: probe_id,
        },
    ))
    .await
    .map_err(|e| format!("submit credential health probe: {e}"))?;
    wait_for_local_encryption_health(
        conn,
        probe_id,
        LocalEncryptionHealth::Healthy,
        "credential health",
    )
    .await?;
    println!("credential_health=ok");

    let fail_closed_id = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::RecordLocalEncryptionHealth {
        request_id: fail_closed_id,
        health: LocalEncryptionHealth::LockedOrInaccessible,
    }))
    .await
    .map_err(|e| format!("submit credential fail-closed health record: {e}"))?;
    wait_for_local_encryption_health(
        conn,
        fail_closed_id,
        LocalEncryptionHealth::LockedOrInaccessible,
        "credential fail-closed",
    )
    .await?;
    println!("fail_closed=ok");

    let reprobe_id = conn.next_request_id();
    conn.command(CoreCommand::Account(
        AccountCommand::ProbeLocalEncryptionHealth {
            request_id: reprobe_id,
        },
    ))
    .await
    .map_err(|e| format!("submit credential health restore probe: {e}"))?;
    wait_for_local_encryption_health(
        conn,
        reprobe_id,
        LocalEncryptionHealth::Healthy,
        "credential health restore",
    )
    .await
}

pub(super) async fn run_native_attention_stage(conn: &mut CoreConnection) -> Result<(), String> {
    let rooms = vec![
        native_attention_room("!message:example.invalid", "Room", false, 8, 8, 0),
        native_attention_room("!dm:example.invalid", "Direct", true, 3, 3, 0),
        native_attention_room("!mention:example.invalid", "Mention", false, 1, 1, 1),
    ];
    let capabilities = native_attention_available_capabilities();
    let attention = native_attention_state_from_rooms(NativeAttentionProjectionInput {
        rooms: &rooms,
        active_room_id: None,
        muted_room_ids: &[],
        room_notification_modes: &std::collections::HashMap::new(),
        ignored_user_ids: &std::collections::BTreeSet::new(),
        window_focused: false,
        observation: NativeAttentionObservationKind::Live,
        previous_candidate: None,
        capabilities,
    });

    let candidate = attention
        .summary
        .candidate
        .as_ref()
        .ok_or_else(|| "native attention candidate was not projected".to_owned())?;
    if candidate.kind != RoomAttentionKind::Mention || attention.summary.badge_count != 12 {
        return Err("native attention candidate priority or badge count was wrong".to_owned());
    }

    let candidate_id = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::UpdateNativeAttentionState {
        request_id: candidate_id,
        attention: attention.clone(),
    }))
    .await
    .map_err(|e| format!("native attention: submit candidate update failed: {e}"))?;
    wait_for_native_attention_state(conn, candidate_id, &attention, "native attention candidate")
        .await?;
    println!("notification_candidate=ok");
    println!("badge_state=ok");

    let focused = native_attention_state_from_rooms(NativeAttentionProjectionInput {
        rooms: &rooms,
        active_room_id: Some("!mention:example.invalid"),
        muted_room_ids: &[],
        room_notification_modes: &std::collections::HashMap::new(),
        ignored_user_ids: &std::collections::BTreeSet::new(),
        window_focused: true,
        observation: NativeAttentionObservationKind::Live,
        previous_candidate: None,
        capabilities,
    });
    if focused.summary.candidate.is_some()
        || focused.dispatch
            != (NativeAttentionDispatchState::Suppressed {
                reason: NativeAttentionSuppressionReason::WindowFocused,
            })
    {
        return Err("native attention focused room suppression was not projected".to_owned());
    }
    println!("suppress_focus=ok");

    let mut notification_modes = std::collections::HashMap::new();
    notification_modes.insert(
        "!message:example.invalid".to_owned(),
        RoomNotificationMode::Mute,
    );
    notification_modes.insert(
        "!dm:example.invalid".to_owned(),
        RoomNotificationMode::Mentions,
    );
    let with_modes = native_attention_state_from_rooms(NativeAttentionProjectionInput {
        rooms: &rooms,
        active_room_id: None,
        muted_room_ids: &[],
        room_notification_modes: &notification_modes,
        ignored_user_ids: &std::collections::BTreeSet::new(),
        window_focused: false,
        observation: NativeAttentionObservationKind::Live,
        previous_candidate: None,
        capabilities,
    });
    if with_modes.summary.unread_count != 1
        || with_modes.summary.highlight_count != 1
        || with_modes.summary.badge_count != 4
        || with_modes
            .summary
            .candidate
            .as_ref()
            .map(|candidate| candidate.kind)
            != Some(RoomAttentionKind::Mention)
    {
        return Err("native attention did not respect per-room notification modes".to_owned());
    }
    println!("room_notification_modes=ok");

    let clear = native_attention_state_from_rooms(NativeAttentionProjectionInput {
        rooms: &[],
        active_room_id: None,
        muted_room_ids: &[],
        room_notification_modes: &std::collections::HashMap::new(),
        ignored_user_ids: &std::collections::BTreeSet::new(),
        window_focused: false,
        observation: NativeAttentionObservationKind::Live,
        previous_candidate: attention.summary.candidate.as_ref(),
        capabilities,
    });
    if clear.summary.badge_count != 0 || clear.summary.candidate.is_some() {
        return Err("native attention clear state retained badge or candidate".to_owned());
    }

    let clear_id = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::UpdateNativeAttentionState {
        request_id: clear_id,
        attention: clear.clone(),
    }))
    .await
    .map_err(|e| format!("native attention: submit clear update failed: {e}"))?;
    wait_for_native_attention_state(conn, clear_id, &clear, "native attention clear").await?;
    println!("clear_badge=ok");

    Ok(())
}

fn native_attention_available_capabilities() -> NativeAttentionCapabilities {
    NativeAttentionCapabilities {
        notifications: NativeAttentionCapability::Available,
        badge: NativeAttentionCapability::Available,
        overlay_icon: NativeAttentionCapability::Available,
        sound: NativeAttentionCapability::Available,
        tray: NativeAttentionCapability::Available,
        activation: NativeAttentionCapability::Available,
    }
}

pub(super) async fn seed_encrypted_room_key_for_qa(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    label: &str,
) -> Result<String, String> {
    let create_room_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::CreateRoom {
        request_id: create_room_id,
        options: private_room_options("QA E2EE Backup Room", true),
    }))
    .await
    .map_err(|e| format!("{label}: submit encrypted room create failed: {e}"))?;

    let room_id = wait_for_room_created(conn, create_room_id, label).await?;

    wait_for_room_in_room_list(conn, &room_id, "room list after encrypted backup seed").await?;

    let key = TimelineKey::room(account_key.clone(), room_id.clone());
    let subscribe_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::Subscribe {
        request_id: subscribe_id,
        key: key.clone(),
        initial_backfill: koushi_protocol::command::InitialBackfillPolicy::Disabled,
    }))
    .await
    .map_err(|e| format!("{label}: submit encrypted timeline subscribe failed: {e}"))?;

    wait_for_initial_items(conn, &key, subscribe_id, "subscribe encrypted backup seed").await?;

    let transaction_id = "qa-e2ee-key-backup-seed".to_owned();
    let send_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::SendText {
        request_id: send_id,
        key: key.clone(),
        transaction_id: transaction_id.clone(),
        document: koushi_state::ComposerDocument::from_plain_text(
            E2EE_KEY_BACKUP_SEED_BODY.to_owned(),
        ),
    }))
    .await
    .map_err(|e| format!("{label}: submit encrypted backup seed send failed: {e}"))?;

    wait_for_send_flow_completion(
        conn,
        send_id,
        &key,
        &transaction_id,
        E2EE_KEY_BACKUP_SEED_BODY,
        "send encrypted backup seed",
    )
    .await?;

    Ok(room_id)
}

pub(super) async fn enable_key_backup_for_qa(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    passphrase: Option<AuthSecret>,
    label: &str,
) -> Result<String, String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Account(AccountCommand::EnableKeyBackup {
        request_id,
        passphrase,
    }))
    .await
    .map_err(|e| format!("{label}: submit enable key backup failed: {e}"))?;

    wait_for_key_backup_enabled(conn, account_key, request_id, label).await
}

async fn wait_for_key_backup_enabled(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    request_id: RequestId,
    label: &str,
) -> Result<String, String> {
    if let KeyBackupStatus::Enabled { version } = &conn.snapshot().e2ee_trust.key_backup {
        return Ok(version.clone());
    }

    loop {
        let event = tokio::time::timeout(E2EE_EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for key backup Enabled"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::E2eeTrust(E2eeTrustEvent::KeyBackupChanged {
                account_key: ev_account_key,
                status,
            }) if &ev_account_key == account_key => match status {
                KeyBackupStatus::Enabled { version } => return Ok(version),
                KeyBackupStatus::Failed {
                    request_id: failed_id,
                    kind,
                } if failed_id == request_id.sequence => {
                    return Err(format!("{label}: key backup enable failed: {kind:?}"));
                }
                _ => {}
            },
            CoreEvent::StateDelta(_) => match conn.snapshot().e2ee_trust.key_backup {
                KeyBackupStatus::Enabled { version } => return Ok(version),
                KeyBackupStatus::Failed {
                    request_id: failed_id,
                    kind,
                } if failed_id == request_id.sequence => {
                    return Err(format!("{label}: key backup enable failed: {kind:?}"));
                }
                _ => {}
            },
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

async fn restore_key_backup_failure_for_qa(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    version: Option<String>,
    label: &str,
) -> Result<(), String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Account(AccountCommand::RestoreKeyBackup {
        request_id,
        version,
        request: RecoveryRequest {
            secret: AuthSecret::new(QA_WRONG_RECOVERY_SECRET),
        },
    }))
    .await
    .map_err(|e| format!("{label}: submit restore key backup failed: {e}"))?;

    wait_for_key_backup_failed(conn, account_key, request_id, label).await
}

async fn restore_key_backup_success_for_qa(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    version: Option<String>,
    secret: AuthSecret,
    label: &str,
) -> Result<(), String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Account(AccountCommand::RestoreKeyBackup {
        request_id,
        version,
        request: RecoveryRequest { secret },
    }))
    .await
    .map_err(|e| format!("{label}: submit restore key backup failed: {e}"))?;

    wait_for_key_backup_restored(conn, account_key, request_id, label).await
}

async fn wait_for_key_backup_failed(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    request_id: RequestId,
    label: &str,
) -> Result<(), String> {
    let mut saw_request_state = matches!(
        conn.snapshot().e2ee_trust.key_backup,
        KeyBackupStatus::Restoring {
            request_id: current,
            ..
        } if current == request_id.sequence
    );

    loop {
        let event = tokio::time::timeout(E2EE_EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for key backup failure"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::E2eeTrust(E2eeTrustEvent::KeyBackupChanged {
                account_key: ev_account_key,
                status,
            }) if &ev_account_key == account_key => match status {
                KeyBackupStatus::Failed {
                    request_id: failed_id,
                    ..
                } if failed_id == request_id.sequence => return Ok(()),
                KeyBackupStatus::Restoring {
                    request_id: current,
                    ..
                } if current == request_id.sequence => {
                    saw_request_state = true;
                }
                KeyBackupStatus::Enabled { .. } if saw_request_state => {
                    return Err(format!("{label}: restore unexpectedly succeeded"));
                }
                _ => {}
            },
            CoreEvent::StateDelta(_) => match conn.snapshot().e2ee_trust.key_backup {
                KeyBackupStatus::Failed {
                    request_id: failed_id,
                    ..
                } if failed_id == request_id.sequence => return Ok(()),
                KeyBackupStatus::Restoring {
                    request_id: current,
                    ..
                } if current == request_id.sequence => {
                    saw_request_state = true;
                }
                KeyBackupStatus::Enabled { .. } if saw_request_state => {
                    return Err(format!("{label}: restore unexpectedly succeeded"));
                }
                _ => {}
            },
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

async fn wait_for_key_backup_restored(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    request_id: RequestId,
    label: &str,
) -> Result<(), String> {
    let mut saw_request_state = matches!(
        conn.snapshot().e2ee_trust.key_backup,
        KeyBackupStatus::Restoring {
            request_id: current,
            ..
        } if current == request_id.sequence
    );
    let mut saw_restored_room = false;

    loop {
        let event = tokio::time::timeout(E2EE_EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for key backup restore success"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::E2eeTrust(E2eeTrustEvent::KeyBackupChanged {
                account_key: ev_account_key,
                status,
            }) if &ev_account_key == account_key => match status {
                KeyBackupStatus::Restoring {
                    request_id: current,
                    restored_rooms,
                    ..
                } if current == request_id.sequence => {
                    saw_request_state = true;
                    saw_restored_room |= restored_rooms > 0;
                }
                KeyBackupStatus::Enabled { .. } if saw_request_state => {
                    if saw_restored_room {
                        return Ok(());
                    }
                    return Err(format!(
                        "{label}: restore succeeded without any joined room"
                    ));
                }
                KeyBackupStatus::Failed {
                    request_id: failed_id,
                    kind,
                } if failed_id == request_id.sequence => {
                    return Err(format!("{label}: key backup restore failed: {kind:?}"));
                }
                _ => {}
            },
            CoreEvent::StateDelta(_) => match conn.snapshot().e2ee_trust.key_backup {
                KeyBackupStatus::Restoring {
                    request_id: current,
                    restored_rooms,
                    ..
                } if current == request_id.sequence => {
                    saw_request_state = true;
                    saw_restored_room |= restored_rooms > 0;
                }
                KeyBackupStatus::Enabled { .. } if saw_request_state => {
                    if saw_restored_room {
                        return Ok(());
                    }
                    return Err(format!(
                        "{label}: restore succeeded without any joined room"
                    ));
                }
                KeyBackupStatus::Failed {
                    request_id: failed_id,
                    kind,
                } if failed_id == request_id.sequence => {
                    return Err(format!("{label}: key backup restore failed: {kind:?}"));
                }
                _ => {}
            },
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

async fn reset_identity_for_qa(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    password: String,
    label: &str,
) -> Result<(), String> {
    let request_id = conn.next_request_id();
    let flow_id = request_id.sequence;
    conn.command(CoreCommand::Account(AccountCommand::ResetIdentity {
        request_id,
    }))
    .await
    .map_err(|e| format!("{label}: submit reset identity failed: {e}"))?;

    match wait_for_identity_reset_auth_or_done(conn, account_key, flow_id, request_id, label)
        .await?
    {
        IdentityResetWait::Completed => Ok(()),
        IdentityResetWait::AuthRequired(IdentityResetAuthType::Uiaa) => {
            let submit_request_id = conn.next_request_id();
            conn.command(CoreCommand::Account(
                AccountCommand::SubmitIdentityResetAuth {
                    request_id: submit_request_id,
                    flow_id,
                    request: IdentityResetAuthRequest::UiaaPassword {
                        password: AuthSecret::new(password),
                    },
                },
            ))
            .await
            .map_err(|e| format!("{label}: submit reset identity UIAA failed: {e}"))?;
            wait_for_identity_reset_done(conn, account_key, flow_id, submit_request_id, label).await
        }
        IdentityResetWait::AuthRequired(IdentityResetAuthType::OAuth) => Err(format!(
            "{label}: OAuth identity reset cannot run headlessly"
        )),
        IdentityResetWait::AuthRequired(IdentityResetAuthType::Unknown) => Err(format!(
            "{label}: unknown identity reset auth type cannot run headlessly"
        )),
    }
}

enum IdentityResetWait {
    Completed,
    AuthRequired(IdentityResetAuthType),
}

async fn wait_for_identity_reset_auth_or_done(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    flow_id: u64,
    command_request_id: RequestId,
    label: &str,
) -> Result<IdentityResetWait, String> {
    let mut saw_request_state = false;

    loop {
        let event = tokio::time::timeout(E2EE_EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for identity reset auth/done"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::E2eeTrust(E2eeTrustEvent::IdentityResetChanged {
                account_key: ev_account_key,
                state,
            }) if &ev_account_key == account_key => {
                if matches!(state, IdentityResetState::Idle) {
                    return Ok(IdentityResetWait::Completed);
                }
                if let Some(result) = identity_reset_observation(&state, flow_id, label)? {
                    return Ok(result);
                }
                if matches!(
                    state,
                    IdentityResetState::Resetting { request_id: current }
                        if current == flow_id
                ) {
                    saw_request_state = true;
                }
            }
            CoreEvent::StateDelta(_) => {
                let state = conn.snapshot().e2ee_trust.identity_reset;
                if !matches!(state, IdentityResetState::Idle) {
                    if let Some(result) = identity_reset_observation(&state, flow_id, label)? {
                        return Ok(result);
                    }
                }
                if matches!(
                    state,
                    IdentityResetState::Resetting { request_id: current }
                        if current == flow_id
                ) {
                    saw_request_state = true;
                }
                if saw_request_state && matches!(state, IdentityResetState::Idle) {
                    return Ok(IdentityResetWait::Completed);
                }
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == command_request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

async fn wait_for_identity_reset_done(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    flow_id: u64,
    command_request_id: RequestId,
    label: &str,
) -> Result<(), String> {
    let mut saw_request_state = matches!(
        conn.snapshot().e2ee_trust.identity_reset,
        IdentityResetState::Resetting {
            request_id: current
        } if current == flow_id
    );

    loop {
        let event = tokio::time::timeout(E2EE_EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for identity reset completion"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::E2eeTrust(E2eeTrustEvent::IdentityResetChanged {
                account_key: ev_account_key,
                state,
            }) if &ev_account_key == account_key => match state {
                IdentityResetState::Idle => return Ok(()),
                IdentityResetState::Resetting {
                    request_id: current,
                } if current == flow_id => {
                    saw_request_state = true;
                }
                IdentityResetState::Failed {
                    request_id: failed_id,
                    kind,
                } if failed_id == flow_id => {
                    return Err(format!("{label}: identity reset failed: {kind:?}"));
                }
                _ => {}
            },
            CoreEvent::StateDelta(_) => match conn.snapshot().e2ee_trust.identity_reset {
                IdentityResetState::Idle if saw_request_state => return Ok(()),
                IdentityResetState::Resetting {
                    request_id: current,
                } if current == flow_id => {
                    saw_request_state = true;
                }
                IdentityResetState::Failed {
                    request_id: failed_id,
                    kind,
                } if failed_id == flow_id => {
                    return Err(format!("{label}: identity reset failed: {kind:?}"));
                }
                _ => {}
            },
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == command_request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

fn identity_reset_observation(
    state: &IdentityResetState,
    request_sequence: u64,
    label: &str,
) -> Result<Option<IdentityResetWait>, String> {
    match state {
        IdentityResetState::AwaitingAuth {
            request_id,
            auth_type,
        } if *request_id == request_sequence => {
            Ok(Some(IdentityResetWait::AuthRequired(*auth_type)))
        }
        IdentityResetState::Failed { request_id, kind } if *request_id == request_sequence => {
            Err(format!("{label}: identity reset failed: {kind:?}"))
        }
        _ => Ok(None),
    }
}

pub(super) async fn verify_second_device_room_key_delivery_for_qa(
    conn_a: &mut CoreConnection,
    conn_a2: &mut CoreConnection,
    account_key_a: &AccountKey,
    account_key_a2: &AccountKey,
    room_id: &str,
) -> Result<(), String> {
    wait_for_room_in_room_list(conn_a, room_id, "A room list before encrypted send").await?;
    wait_for_room_in_room_list(conn_a2, room_id, "A2 room list before encrypted receive").await?;

    let key_a = TimelineKey::room(account_key_a.clone(), room_id.to_owned());
    let key_a2 = TimelineKey::room(account_key_a2.clone(), room_id.to_owned());

    let subscribe_a2_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Timeline(TimelineCommand::Subscribe {
            request_id: subscribe_a2_id,
            key: key_a2.clone(),
            initial_backfill: koushi_protocol::command::InitialBackfillPolicy::Disabled,
        }))
        .await
        .map_err(|e| format!("second-device decrypt: submit A2 subscribe failed: {e}"))?;

    let initial_a2 = wait_for_initial_items(
        conn_a2,
        &key_a2,
        subscribe_a2_id,
        "second-device encrypted room subscribe",
    )
    .await?;
    assert_no_decryption_failure_items(&initial_a2, "second-device encrypted room initial")?;
    if find_timeline_item_with_body(&initial_a2, E2EE_KEY_BACKUP_SEED_BODY).is_none() {
        return Err("second-device decrypt: restored backup seed body was not visible".to_owned());
    }

    let transaction_id = "qa-e2ee-second-device-delivery".to_owned();
    let send_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::SendText {
            request_id: send_id,
            key: key_a.clone(),
            transaction_id: transaction_id.clone(),
            document: koushi_state::ComposerDocument::from_plain_text(
                E2EE_SECOND_DEVICE_BODY.to_owned(),
            ),
        }))
        .await
        .map_err(|e| format!("second-device decrypt: submit encrypted send failed: {e}"))?;

    wait_for_send_flow_completion(
        conn_a,
        send_id,
        &key_a,
        &transaction_id,
        E2EE_SECOND_DEVICE_BODY,
        "second-device encrypted send",
    )
    .await?;

    wait_for_item_with_body_or_decryption_failure(
        conn_a2,
        &key_a2,
        E2EE_SECOND_DEVICE_BODY,
        "second-device encrypted receive",
    )
    .await?;

    Ok(())
}

pub(super) async fn verify_multi_user_multi_device_room_key_delivery_for_qa(
    config: &QaConfig,
    conn_a: &mut CoreConnection,
    conn_a2: &mut CoreConnection,
    account_key_a: &AccountKey,
    account_key_a2: &AccountKey,
    recipient_base: Option<(&mut CoreConnection, &AccountKey)>,
) -> Result<(), String> {
    let check_recipient_second_device = env_flag_enabled(ENV_E2EE_RECIPIENT_SECOND_DEVICE)?;
    let user_b_full_id = format!("@{}:{}", config.user_b, config.server_name);
    let room_id = create_room_for_qa(
        conn_a,
        "QA E2EE Multi Device DM",
        true,
        "e2ee multi-device create encrypted room",
    )
    .await?;

    wait_for_room_in_room_list(
        conn_a,
        &room_id,
        "e2ee multi-device A room list after create",
    )
    .await?;

    invite_user_for_qa(
        conn_a,
        &room_id,
        &user_b_full_id,
        "e2ee multi-device invite B",
    )
    .await?;

    let mut recipient = match recipient_base {
        Some((conn, account_key)) => QaE2eeRecipient::Borrowed { conn, account_key },
        None => QaE2eeRecipient::Owned(
            login_synced_participant_for_qa(
                &config.homeserver,
                qa_data_dir("e2ee-b"),
                &config.user_b,
                &config.password_b,
                DEVICE_B,
                "e2ee login B",
                "gate-bootstrap-b",
                QaParticipantLoginGate::BootstrapNewIdentity,
            )
            .await?
            .into(),
        ),
    };
    let mut owned_recipient_second_device = None;
    let mut owned_unverified_recipient_device = None;
    let stage_result: Result<(), String> = async {
        let (conn_b, account_key_b) = recipient.connection_and_account_key();

        wait_for_invite_in_snapshot(
            conn_b,
            &room_id,
            Some(false),
            "e2ee multi-device wait for B invite",
        )
        .await?;
        accept_invite_for_qa(conn_b, &room_id, "e2ee multi-device B accepts invite").await?;

        let settings_a = load_room_settings_for_qa(
            conn_a,
            &room_id,
            "e2ee multi-device A observes B membership",
        )
        .await?;
        assert_room_settings_contains_members(
            &settings_a,
            &[account_key_a.0.as_str(), user_b_full_id.as_str()],
            "e2ee multi-device A observes B membership",
        )?;
        wait_for_room_in_room_list(
            conn_a2,
            &room_id,
            "e2ee multi-device A2 room list after create",
        )
        .await?;
        wait_for_room_in_room_list(conn_b, &room_id, "e2ee multi-device B room list").await?;

        let key_a = TimelineKey::room(account_key_a.clone(), room_id.clone());
        let key_a2 = TimelineKey::room(account_key_a2.clone(), room_id.clone());
        let key_b = TimelineKey::room(account_key_b.clone(), room_id.clone());

        let initial_a =
            subscribe_timeline_for_qa(conn_a, &key_a, "e2ee multi-device subscribe A").await?;
        let initial_a2 =
            subscribe_timeline_for_qa(conn_a2, &key_a2, "e2ee multi-device subscribe A2").await?;
        let initial_b =
            subscribe_timeline_for_qa(conn_b, &key_b, "e2ee multi-device subscribe B").await?;
        assert_no_decryption_failure_items(&initial_a, "e2ee multi-device A initial")?;
        assert_no_decryption_failure_items(&initial_a2, "e2ee multi-device A2 initial")?;
        assert_no_decryption_failure_items(&initial_b, "e2ee multi-device B initial")?;

        let mut recipient_second_device_key = None;
        if check_recipient_second_device {
            let runtime_b2 = start_isolated_qa_runtime("e2ee-b2")?;
            let conn_b2 = runtime_b2.attach();
            owned_recipient_second_device =
                Some(QaOwnedRuntimeParticipant::new(runtime_b2, conn_b2));
            let participant_b2 = owned_recipient_second_device
                .as_mut()
                .expect("B2 owner was installed before login");
            let login_b2 = participant_b2.conn.next_request_id();
            participant_b2
                .conn
                .command(CoreCommand::Account(AccountCommand::LoginPassword {
                    request_id: login_b2,
                    request: koushi_state::LoginRequest {
                        homeserver: config.homeserver.clone(),
                        username: config.user_b.clone(),
                        password: AuthSecret::new(config.password_b.clone()),
                        device_display_name: Some("Koushi Core QA B2".to_owned()),
                    },
                    platform: koushi_state::DisplayPlatform::Linux,
                }))
                .await
                .map_err(|error| format!("e2ee login B2 submit: {error}"))?;
            participant_b2.mark_login_submitted();
            let session_b2 =
                wait_for_existing_identity_gate(&mut participant_b2.conn, "e2ee B2 gate").await?;
            let session_b =
                authenticated_session_info(conn_b, "session B info for E2EE multi-device")?;
            verify_provisional_second_device_for_qa(
                conn_b,
                &mut participant_b2.conn,
                &session_b,
                &session_b2,
                "e2ee recipient verification B/B2",
                SasQaOutcome::Success,
            )
            .await?;
            let account_key_b2 =
                wait_for_logged_in(&mut participant_b2.conn, login_b2, "e2ee login B2").await?;
            participant_b2.mark_logged_in(account_key_b2.clone());
            wait_for_ready_snapshot(&mut participant_b2.conn, "e2ee B2 Ready").await?;
            start_sync_for_qa(&mut participant_b2.conn, "e2ee B2 sync").await?;
            wait_for_room_in_room_list(
                &mut participant_b2.conn,
                &room_id,
                "e2ee multi-device B2 room list",
            )
            .await?;
            let key_b2 = TimelineKey::room(account_key_b2.clone(), room_id.clone());
            let initial_b2 = subscribe_timeline_for_qa(
                &mut participant_b2.conn,
                &key_b2,
                "e2ee multi-device subscribe B2",
            )
            .await?;
            assert_no_decryption_failure_items(&initial_b2, "e2ee multi-device B2 initial")?;
            recipient_second_device_key = Some(key_b2);
        }

        let runtime_b3 = start_isolated_qa_runtime("e2ee-b3-unverified")?;
        let conn_b3 = runtime_b3.attach();
        owned_unverified_recipient_device =
            Some(QaOwnedRuntimeParticipant::new(runtime_b3, conn_b3));
        let participant_b3 = owned_unverified_recipient_device
            .as_mut()
            .expect("B3 owner was installed before login");
        let login_b3 = participant_b3.conn.next_request_id();
        participant_b3
            .conn
            .command(CoreCommand::Account(AccountCommand::LoginPassword {
                request_id: login_b3,
                request: koushi_state::LoginRequest {
                    homeserver: config.homeserver.clone(),
                    username: config.user_b.clone(),
                    password: AuthSecret::new(config.password_b.clone()),
                    device_display_name: Some("Koushi Core QA B3 Unverified".to_owned()),
                },
                platform: koushi_state::DisplayPlatform::Linux,
            }))
            .await
            .map_err(|error| format!("e2ee unverified peer login submit: {error}"))?;
        participant_b3.mark_login_submitted();
        let session_b3 =
            wait_for_existing_identity_gate(&mut participant_b3.conn, "e2ee unverified peer gate")
                .await?;
        refresh_device_keys_and_assert_known_for_qa(
            conn_a,
            VerificationTarget {
                user_id: session_b3.user_id.clone(),
                device_id: session_b3.device_id.clone(),
            },
            "e2ee unverified peer device discovery",
        )
        .await?;
        let transaction_id = "qa-e2ee-multi-user-multi-device-delivery".to_owned();
        let send_id = conn_a.next_request_id();
        conn_a
            .command(CoreCommand::Timeline(TimelineCommand::SendText {
                request_id: send_id,
                key: key_a.clone(),
                transaction_id: transaction_id.clone(),
                document: koushi_state::ComposerDocument::from_plain_text(
                    E2EE_MULTI_USER_MULTI_DEVICE_BODY.to_owned(),
                ),
            }))
            .await
            .map_err(|e| format!("e2ee multi-device: submit encrypted send failed: {e}"))?;

        wait_for_send_flow_completion_with_timeout(
            conn_a,
            send_id,
            &key_a,
            &transaction_id,
            E2EE_MULTI_USER_MULTI_DEVICE_BODY,
            "e2ee multi-device encrypted send",
            E2EE_EVENT_TIMEOUT,
        )
        .await?;
        println!("e2ee_unverified_peer_send_nonblocking=ok");

        wait_for_item_with_body_or_decryption_failure(
            conn_a2,
            &key_a2,
            E2EE_MULTI_USER_MULTI_DEVICE_BODY,
            "e2ee multi-device A2 receive",
        )
        .await?;
        wait_for_item_with_body_or_decryption_failure(
            conn_b,
            &key_b,
            E2EE_MULTI_USER_MULTI_DEVICE_BODY,
            "e2ee multi-device B receive",
        )
        .await?;

        let session_b = authenticated_session_info(conn_b, "blocked QA B session")?;
        verify_provisional_second_device_for_qa(
            conn_b,
            &mut participant_b3.conn,
            &session_b,
            &session_b3,
            "blocked QA promote B3",
            SasQaOutcome::Success,
        )
        .await?;
        let account_key_b3 =
            wait_for_logged_in(&mut participant_b3.conn, login_b3, "blocked QA B3 login").await?;
        participant_b3.mark_logged_in(account_key_b3.clone());
        wait_for_ready_snapshot(&mut participant_b3.conn, "blocked QA B3 Ready").await?;
        start_sync_for_qa(&mut participant_b3.conn, "blocked QA B3 sync").await?;
        wait_for_room_in_room_list(&mut participant_b3.conn, &room_id, "blocked QA B3 room")
            .await?;
        let key_b3 = TimelineKey::room(account_key_b3.clone(), room_id.clone());
        let initial_b3 =
            subscribe_timeline_for_qa(&mut participant_b3.conn, &key_b3, "blocked QA B3 timeline")
                .await?;

        tokio::time::timeout(
            EVENT_TIMEOUT,
            conn_a.qa_set_local_device_blacklisted(
                VerificationTarget {
                    user_id: session_b3.user_id.clone(),
                    device_id: session_b3.device_id.clone(),
                },
                room_id.clone(),
            ),
        )
        .await
        .map_err(|_| "blocked QA blacklist ack timeout".to_owned())?
        .map_err(|_| "blocked QA blacklist failed".to_owned())?;
        let blocked_body = "Koushi blocked-device withheld probe";
        let blocked_txn = "qa-e2ee-blocked-device-withheld".to_owned();
        let blocked_send = conn_a.next_request_id();
        conn_a
            .command(CoreCommand::Timeline(TimelineCommand::SendText {
                request_id: blocked_send,
                key: key_a.clone(),
                transaction_id: blocked_txn.clone(),
                document: koushi_state::ComposerDocument::from_plain_text(blocked_body.to_owned()),
            }))
            .await
            .map_err(|_| "blocked QA Core send submit failed".to_owned())?;
        let blocked_send_outcome = wait_for_send_flow_completion_with_timeout(
            conn_a,
            blocked_send,
            &key_a,
            &blocked_txn,
            blocked_body,
            "blocked QA Core send",
            E2EE_EVENT_TIMEOUT,
        )
        .await?;
        wait_for_item_with_body_or_decryption_failure(
            conn_b,
            &key_b,
            blocked_body,
            "blocked QA nonblocked receive",
        )
        .await?;
        wait_for_withheld_event_projection_from_source(
            &mut participant_b3.conn,
            &key_b3,
            &blocked_send_outcome.event_id,
            blocked_body,
            &initial_b3,
            "blocked QA B3 withheld event",
            E2EE_EVENT_TIMEOUT,
        )
        .await?;
        println!("e2ee_blocked_device_withheld=ok");

        if let Some(key_b2) = recipient_second_device_key {
            let participant_b2 = owned_recipient_second_device
                .as_mut()
                .expect("B2 key exists only while its owner is retained");
            wait_for_item_with_body_or_decryption_failure(
                &mut participant_b2.conn,
                &key_b2,
                E2EE_MULTI_USER_MULTI_DEVICE_BODY,
                "e2ee multi-device B2 receive",
            )
            .await?;
            println!("e2ee_recipient_second_device_decrypt=ok");
        }

        Ok(())
    }
    .await;

    finish_e2ee_recipient_stage_with_owned_cleanup(
        stage_result,
        Some((
            recipient.into_owned(),
            owned_recipient_second_device,
            owned_unverified_recipient_device,
        )),
        cleanup_e2ee_multi_device_participants,
    )
    .await
}

fn assert_no_decryption_failure_items(items: &[TimelineItem], label: &str) -> Result<(), String> {
    if items.iter().any(timeline_item_is_decryption_failure) {
        return Err(format!(
            "{label}: timeline contained an undecryptable event"
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;
