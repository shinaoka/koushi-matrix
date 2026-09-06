use super::*;

fn ready_state(room_id: &str) -> koushi_state::AppState {
    let mut state = koushi_state::AppState::default();
    state.session = koushi_state::SessionState::Ready(koushi_state::SessionInfo {
        homeserver: "https://example.invalid".to_owned(),
        user_id: "@alice:example.invalid".to_owned(),
        device_id: "DEVICE".to_owned(),
        authentication_method: Default::default(),
    });
    state.timeline.room_id = Some(room_id.to_owned());
    state
}

#[tokio::test]
async fn timeline_wait_wrappers_delegate_to_core_outcome_service() {
    let room_id = "!room:example.invalid";
    let account_key = AccountKey("@alice:example.invalid".to_owned());
    let target = koushi_state::ComposerTarget::Main {
        room_id: room_id.to_owned(),
    };

    let (mut connection, control) = CoreConnection::new_for_testing(8);
    let request_id = connection.next_request_id();
    let mut accepted_state = ready_state(room_id);
    accepted_state
        .composer_drafts
        .apply_room_draft(
            room_id.to_owned(),
            koushi_state::ComposerDocument::from_plain_text("draft"),
            3.into(),
        )
        .expect("revision");
    control.send_snapshot(koushi_protocol::state_update::VersionedAppStateSnapshot {
        generation: 2,
        state: accepted_state,
    });
    let (revision, generation) = wait_for_composer_draft_acceptance(
        &mut connection,
        request_id,
        account_key.clone(),
        target.clone(),
        3.into(),
        1,
    )
    .await
    .expect("composer wrapper");
    assert_eq!(revision, 3.into());
    assert_eq!(generation, 2);

    let (mut connection, control) = CoreConnection::new_for_testing(8);
    let request_id = connection.next_request_id();
    let submission_id = SubmissionId::new("submission");
    control.send_snapshot(koushi_protocol::state_update::VersionedAppStateSnapshot {
        generation: 2,
        state: ready_state(room_id),
    });
    let settlement = wait_for_submission_settlement(
        &mut connection,
        request_id,
        account_key.clone(),
        target.clone(),
        submission_id.clone(),
        1,
    );
    tokio::pin!(settlement);
    control.send_event(CoreEvent::Timeline(TimelineEvent::SubmissionAccepted {
        request_id,
        key: TimelineKey::room(account_key.clone(), room_id),
        submission_id: submission_id.clone(),
        transaction_id: "txn".to_owned(),
    }));
    let response = settlement.await.expect("submission wrapper");
    assert_eq!(response.submission_id, submission_id);
    assert_eq!(response.transaction_id.as_deref(), Some("txn"));
}
