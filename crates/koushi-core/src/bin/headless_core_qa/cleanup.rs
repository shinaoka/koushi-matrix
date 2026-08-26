use super::event_wait::{
    QaEventDeadline, wait_for_logged_out, wait_for_operation_failed_and_signed_out,
    wait_for_ready_snapshot, wait_for_session_restored, wait_for_sync_stopped,
};
use super::participants::{
    QaOwnedRuntimeParticipant, QaParticipantLoginOutcome,
    cleanup_owned_e2ee_participant_best_effort,
};
use super::{
    AccountCommand, AccountKey, CoreCommand, CoreConnection, CoreEvent, CoreFailure, CoreRuntime,
    Future, RoomCommand, RoomEvent, SyncCommand,
};

pub(super) async fn leave_e2ee_login_store_room(
    conn: &mut CoreConnection,
    room_id: &str,
    label: &str,
) -> Result<(), String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::LeaveRoom {
        request_id,
        room_id: room_id.to_owned(),
    }))
    .await
    .map_err(|_| format!("{label}: submit room leave failed"))?;

    let deadline = QaEventDeadline::after(super::registry::E2EE_EVENT_TIMEOUT);
    loop {
        let event = deadline
            .recv(conn)
            .await
            .map_err(|_| format!("{label}: room leave timed out"))?
            .map_err(|_| format!("{label}: room leave event stream failed"))?;
        match event {
            CoreEvent::Room(RoomEvent::RoomLeft {
                request_id: event_request_id,
                room_id: event_room_id,
            }) if event_request_id == request_id && event_room_id == room_id => return Ok(()),
            CoreEvent::OperationFailed {
                request_id: event_request_id,
                ..
            } if event_request_id == request_id => {
                return Err(format!("{label}: room leave operation failed"));
            }
            _ => {}
        }
    }
}

pub(super) async fn cleanup_after_login_sync(
    mut conn_a: CoreConnection,
    runtime_a: CoreRuntime,
    data_dir_a: std::path::PathBuf,
    account_key_a: AccountKey,
) -> Result<String, String> {
    let sync_stop_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Sync(SyncCommand::Stop {
            request_id: sync_stop_id,
        }))
        .await
        .map_err(|e| format!("submit sync stop A: {e}"))?;

    wait_for_sync_stopped(&mut conn_a, sync_stop_id, "sync stop A").await?;
    println!("sync_a=stopped");
    drop(conn_a);
    runtime_a.shutdown().await;

    let runtime_a2 = CoreRuntime::start_with_data_dir(data_dir_a);
    let mut conn_a2 = runtime_a2.attach();

    let restore_a_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Account(AccountCommand::RestoreSession {
            request_id: restore_a_id,
            account_key: account_key_a.clone(),
        }))
        .await
        .map_err(|e| format!("submit restore A: {e}"))?;

    wait_for_session_restored(&mut conn_a2, restore_a_id, &account_key_a, "restore A").await?;
    wait_for_ready_snapshot(&mut conn_a2, "restored session A Ready").await?;
    println!("gate_verified_restore=ok");

    let logout_a_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Account(AccountCommand::Logout {
            request_id: logout_a_id,
        }))
        .await
        .map_err(|e| format!("submit logout A: {e}"))?;

    wait_for_logged_out(&mut conn_a2, logout_a_id, &account_key_a, "logout A").await?;

    let restore_preserved_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Account(AccountCommand::RestoreSession {
            request_id: restore_preserved_id,
            account_key: account_key_a.clone(),
        }))
        .await
        .map_err(|e| format!("submit post-logout restore A: {e}"))?;

    wait_for_session_restored(
        &mut conn_a2,
        restore_preserved_id,
        &account_key_a,
        "post-logout explicit restore A",
    )
    .await?;
    wait_for_ready_snapshot(&mut conn_a2, "post-logout explicit restore A Ready").await?;

    let restored_logout_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Account(AccountCommand::Logout {
            request_id: restored_logout_id,
        }))
        .await
        .map_err(|e| format!("submit restored logout A: {e}"))?;
    wait_for_logged_out(
        &mut conn_a2,
        restored_logout_id,
        &account_key_a,
        "restored logout A",
    )
    .await?;
    drop(conn_a2);
    runtime_a2.shutdown().await;
    println!("restore_cleanup=ok");
    Ok("restore_cleanup=ok".to_owned())
}

pub(super) async fn cleanup_logged_in_runtime(
    mut conn: CoreConnection,
    runtime: CoreRuntime,
    account_key: AccountKey,
    label: &str,
) -> Result<(), String> {
    let sync_stop_id = conn.next_request_id();
    conn.command(CoreCommand::Sync(SyncCommand::Stop {
        request_id: sync_stop_id,
    }))
    .await
    .map_err(|e| format!("{label}: submit sync stop failed: {e}"))?;
    wait_for_sync_stopped(&mut conn, sync_stop_id, label).await?;

    let logout_id = conn.next_request_id();
    conn.command(CoreCommand::Account(AccountCommand::Logout {
        request_id: logout_id,
    }))
    .await
    .map_err(|e| format!("{label}: submit logout failed: {e}"))?;
    wait_for_logged_out(&mut conn, logout_id, &account_key, label).await?;

    drop(conn);
    runtime.shutdown().await;
    Ok(())
}

pub(super) async fn cleanup_e2ee_callers_after_stage_failure(
    callers: (QaOwnedRuntimeParticipant, QaOwnedRuntimeParticipant),
) -> Result<(), String> {
    let (caller_a, caller_b) = callers;
    let cleanup_a =
        cleanup_owned_e2ee_participant_best_effort(caller_a, "all E2EE failure cleanup caller A")
            .await;
    let cleanup_b =
        cleanup_owned_e2ee_participant_best_effort(caller_b, "all E2EE failure cleanup caller B")
            .await;

    match (cleanup_a, cleanup_b) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(_), Ok(())) => Err("all E2EE caller A cleanup failed".to_owned()),
        (Ok(()), Err(_)) => Err("all E2EE caller B cleanup failed".to_owned()),
        (Err(_), Err(_)) => Err("all E2EE caller cleanup failed for both participants".to_owned()),
    }
}

pub(super) async fn cleanup_e2ee_multi_device_participants(
    participants: (
        Option<QaOwnedRuntimeParticipant>,
        Option<QaOwnedRuntimeParticipant>,
        Option<QaOwnedRuntimeParticipant>,
    ),
) -> Result<(), String> {
    let (base, second_device, unverified_device) = participants;
    cleanup_all_owned_e2ee_participants(
        [
            unverified_device.map(|participant| (participant, "e2ee cleanup B3")),
            second_device.map(|participant| (participant, "e2ee cleanup B2")),
            base.map(|participant| (participant, "e2ee cleanup B")),
        ],
        |(participant, label)| async move {
            cleanup_owned_e2ee_participant_best_effort(participant, label).await
        },
    )
    .await
}

async fn cleanup_all_owned_e2ee_participants<Participant, Cleanup, CleanupFuture, const N: usize>(
    participants: [Option<Participant>; N],
    mut cleanup: Cleanup,
) -> Result<(), String>
where
    Cleanup: FnMut(Participant) -> CleanupFuture,
    CleanupFuture: Future<Output = Result<(), String>>,
{
    let mut failed = 0usize;
    for participant in participants.into_iter().flatten() {
        if cleanup(participant).await.is_err() {
            failed += 1;
        }
    }

    if failed == 0 {
        Ok(())
    } else {
        Err(format!(
            "E2EE cleanup failed for {failed} owned recipient participant(s)"
        ))
    }
}

pub(super) async fn cleanup_normal_secondary_participant_for_qa(
    normal_secondary: &mut Option<QaParticipantLoginOutcome>,
    label: &str,
) -> Result<(), String> {
    let Some(participant) = normal_secondary.take() else {
        return Ok(());
    };
    cleanup_logged_in_runtime(
        participant.conn,
        participant.runtime,
        participant.account_key,
        label,
    )
    .await
}

pub(super) async fn cleanup_after_full_flow(
    mut conn_a: CoreConnection,
    mut conn_b: CoreConnection,
    runtime_a: CoreRuntime,
    runtime_b: CoreRuntime,
    data_dir_a: std::path::PathBuf,
    account_key_a: AccountKey,
    account_key_b: AccountKey,
) -> Result<String, String> {
    let sync_stop_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Sync(SyncCommand::Stop {
            request_id: sync_stop_id,
        }))
        .await
        .map_err(|e| format!("submit sync stop A: {e}"))?;

    wait_for_sync_stopped(&mut conn_a, sync_stop_id, "sync stop A").await?;
    println!("sync_a=stopped");

    drop(conn_a);
    runtime_a.shutdown().await;

    let runtime_a2 = CoreRuntime::start_with_data_dir(data_dir_a);
    let mut conn_a2 = runtime_a2.attach();

    let restore_a_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Account(AccountCommand::RestoreSession {
            request_id: restore_a_id,
            account_key: account_key_a.clone(),
        }))
        .await
        .map_err(|e| format!("submit restore A: {e}"))?;

    wait_for_session_restored(&mut conn_a2, restore_a_id, &account_key_a, "restore A").await?;
    wait_for_ready_snapshot(&mut conn_a2, "restored session A Ready").await?;

    let logout_a_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Account(AccountCommand::Logout {
            request_id: logout_a_id,
        }))
        .await
        .map_err(|e| format!("submit logout A: {e}"))?;

    wait_for_logged_out(&mut conn_a2, logout_a_id, &account_key_a, "logout A").await?;

    let restore_preserved_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Account(AccountCommand::RestoreSession {
            request_id: restore_preserved_id,
            account_key: account_key_a.clone(),
        }))
        .await
        .map_err(|e| format!("submit post-logout restore A: {e}"))?;

    wait_for_session_restored(
        &mut conn_a2,
        restore_preserved_id,
        &account_key_a,
        "post-logout explicit restore A",
    )
    .await?;
    wait_for_ready_snapshot(&mut conn_a2, "post-logout explicit restore A Ready").await?;

    let restored_logout_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Account(AccountCommand::Logout {
            request_id: restored_logout_id,
        }))
        .await
        .map_err(|e| format!("submit restored logout A: {e}"))?;
    wait_for_logged_out(
        &mut conn_a2,
        restored_logout_id,
        &account_key_a,
        "restored logout A",
    )
    .await?;
    let logout_b_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Account(AccountCommand::Logout {
            request_id: logout_b_id,
        }))
        .await
        .map_err(|e| format!("submit logout B: {e}"))?;

    wait_for_logged_out(&mut conn_b, logout_b_id, &account_key_b, "logout B").await?;

    let restore_last_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Account(AccountCommand::RestoreLastSession {
            request_id: restore_last_id,
        }))
        .await
        .map_err(|e| format!("submit post-logout restore-last: {e}"))?;

    let failure = wait_for_operation_failed_and_signed_out(
        &mut conn_b,
        restore_last_id,
        "post-logout restore-last (must be not-found)",
    )
    .await?;
    if failure != CoreFailure::SessionNotFound {
        return Err(format!(
            "post-logout restore-last failed with unexpected kind: {failure:?}"
        ));
    }
    drop(conn_b);
    runtime_b.shutdown().await;

    println!("restore_cleanup=ok");
    Ok("restore_cleanup=ok".to_owned())
}

#[cfg(test)]
#[path = "cleanup_tests.rs"]
mod tests;
