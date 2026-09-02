use super::*;
use std::collections::BTreeMap;

use koushi_protocol::event::{AccountEvent, RoomEvent, TimelineEvent};
use koushi_state::{
    DisplaySettings, RoomSummary, RoomTags, SessionInfo, SettingsPatch, SpaceMemberEntry,
    SpaceMemberMembership, SpaceMembersProjection, UserProfile,
};

#[test]
fn persisted_read_receipt_policy_seeds_account_runtime_before_session_spawn() {
    let mut state = AppState::default();
    state.settings.values.notifications.send_read_receipts = false;
    assert!(!initial_send_read_receipts(&state));
    state.settings.values.notifications.send_read_receipts = true;
    assert!(initial_send_read_receipts(&state));
}

fn closed_forward_space_member_entry(
    user_id: &str,
    membership: SpaceMemberMembership,
) -> SpaceMemberEntry {
    SpaceMemberEntry {
        user_id: user_id.to_owned(),
        display_name: Some("Closed forward test user".to_owned()),
        display_label: "Closed forward test user".to_owned(),
        original_display_label: "Closed forward test user".to_owned(),
        avatar_url: None,
        power_level: Some(0),
        role: koushi_state::RoomMemberRole::User,
        membership,
        child_room_ids: Vec::new(),
        invite_pending: false,
        role_options: if matches!(membership, SpaceMemberMembership::SpaceJoined) {
            vec![koushi_state::SpaceMemberRoleOption {
                power_level: 50,
                role: koushi_state::RoomMemberRole::Moderator,
                requires_confirmation: false,
            }]
        } else {
            Vec::new()
        },
    }
}

fn closed_forward_space_member_fixture(
    space_id: &str,
    generation: u64,
    user_id: &str,
    membership: SpaceMemberMembership,
) -> Vec<AppAction> {
    let entry = closed_forward_space_member_entry(user_id, membership);
    let (space_joined, space_invited, child_room_only) = match membership {
        SpaceMemberMembership::SpaceJoined => (vec![entry], Vec::new(), Vec::new()),
        SpaceMemberMembership::SpaceInvited => (Vec::new(), vec![entry], Vec::new()),
        SpaceMemberMembership::ChildRoomOnly => (Vec::new(), Vec::new(), vec![entry]),
    };
    vec![
        AppAction::AppStarted,
        AppAction::RestoreSessionSucceeded(SessionInfo {
            homeserver: "https://example.invalid".to_owned(),
            user_id: "@closed-forward-self:example.invalid".to_owned(),
            device_id: "DEVICE".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        AppAction::CurrentDeviceTrustChanged(koushi_state::CurrentDeviceTrustState::Verified),
        AppAction::NavigationLoaded {
            navigation: NavigationState {
                active_space_id: Some(space_id.to_owned()),
                ..NavigationState::default()
            },
        },
        AppAction::SpaceMembersLoadRequested {
            request_id: 1,
            space_id: space_id.to_owned(),
            generation,
        },
        AppAction::SpaceMembersLoaded {
            request_id: 1,
            projection: SpaceMembersProjection {
                space_id: space_id.to_owned(),
                generation,
                space_joined,
                space_invited,
                child_room_only,
                child_room_count: 0,
                complete_child_room_count: 0,
                incomplete_child_room_count: 0,
                power_levels_revision: None,
                can_edit_roles: matches!(membership, SpaceMemberMembership::SpaceJoined),
            },
        },
    ]
}

async fn wait_for_runtime_snapshot(
    connection: &mut CoreConnection,
    predicate: impl Fn(&AppState) -> bool,
) -> AppState {
    // Content/events are the causal barrier; this timeout is only a deadlock watchdog.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let snapshot = connection.snapshot();
            if predicate(&snapshot) {
                return snapshot;
            }
            connection
                .next_versioned_snapshot()
                .await
                .expect("runtime snapshot stream should remain open");
        }
    })
    .await
    .expect("runtime state should reach the expected operation boundary")
}

async fn close_account_actor_for_runtime_test(runtime: &CoreRuntime) {
    let (acknowledged_tx, acknowledged_rx) = oneshot::channel();
    assert!(
        runtime
            .account_actor_test_handle
            .send(AccountMessage::ShutdownWithAck {
                acknowledged: acknowledged_tx,
            })
            .await
    );
    acknowledged_rx
        .await
        .expect("AccountActor closed-channel test acknowledgement");
}

async fn run_closed_space_member_forwarding_case(
    membership: SpaceMemberMembership,
    command: impl FnOnce(RequestId) -> koushi_protocol::command::RoomCommand,
) -> (AppState, CoreFailure, u64) {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let runtime = CoreRuntime::start_with_event_capacity(64);
    let mut connection = runtime.attach();
    let space_id = "!closed-forward-space:example.invalid";
    let user_id = "@closed-forward-user:example.invalid";
    let generation = 9;

    runtime
        .inject_actions(closed_forward_space_member_fixture(
            space_id, generation, user_id, membership,
        ))
        .await;
    wait_for_runtime_snapshot(&mut connection, |snapshot| {
        snapshot.space_members.selected_space_id.as_deref() == Some(space_id)
            && snapshot.space_members.generation == generation
            && matches!(
                snapshot.space_members.operation,
                koushi_state::SpaceMembersOperationState::Idle
            )
    })
    .await;
    // Phase C publishes state before post-commit work. Queue an existing
    // test-hook mutation behind the fixture batch and await its completion
    // before deliberately closing the AccountActor transport.
    runtime
        .inject_composer_drafts_and_wait_for_testing(connection.snapshot().composer_drafts.clone())
        .await;
    // Re-apply the explicit fixture navigation after the real persisted-load
    // stage so command admission observes the intended selected Space.
    runtime
        .inject_actions(vec![AppAction::NavigationLoaded {
            navigation: NavigationState {
                active_space_id: Some(space_id.to_owned()),
                ..NavigationState::default()
            },
        }])
        .await;
    wait_for_runtime_snapshot(&mut connection, |snapshot| {
        snapshot.space_members.selected_space_id.as_deref() == Some(space_id)
            && matches!(
                snapshot.space_members.operation,
                koushi_state::SpaceMembersOperationState::Idle
            )
    })
    .await;

    close_account_actor_for_runtime_test(&runtime).await;
    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Room(command(request_id)))
        .await
        .expect("closed-channel command should enter AppActor");

    let failure = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            match connection
                .recv_event()
                .await
                .expect("runtime event stream should remain open")
            {
                CoreEvent::OperationFailed {
                    request_id: failed_request_id,
                    failure,
                } if failed_request_id == request_id => break failure,
                _ => {}
            }
        }
    })
    .await
    .expect("closed actor forwarding should emit a correlated failure");
    let final_state = wait_for_runtime_snapshot(&mut connection, |snapshot| {
        match snapshot.space_members.operation {
            koushi_state::SpaceMembersOperationState::Failed {
                request_id: failed_request_id,
                ..
            }
            | koushi_state::SpaceMembersOperationState::RoleUpdateFailed {
                request_id: failed_request_id,
                ..
            } => failed_request_id == request_id.sequence,
            _ => false,
        }
    })
    .await;

    let debug = format!("{:?}", final_state.space_members);
    let diagnostics = serde_json::to_string(&koushi_diagnostics::snapshot())
        .expect("diagnostics should serialize");
    for private_value in [space_id, user_id] {
        assert!(!debug.contains(private_value), "{debug}");
        assert!(!diagnostics.contains(private_value), "{diagnostics}");
    }

    runtime.shutdown_handle().abort();
    runtime.media_lifecycle.abort();
    (final_state, failure, request_id.sequence)
}

#[tokio::test]
async fn closed_account_forwarding_rolls_back_space_member_load() {
    let (state, failure, request_id) =
        run_closed_space_member_forwarding_case(SpaceMemberMembership::SpaceJoined, |request_id| {
            koushi_protocol::command::RoomCommand::LoadSpaceMembers {
                request_id,
                space_id: "!closed-forward-space:example.invalid".to_owned(),
                generation: 9,
            }
        })
        .await;

    assert_eq!(
        failure,
        CoreFailure::RoomOperationFailed {
            kind: RoomFailureKind::Sdk
        }
    );
    assert!(matches!(
        state.space_members.operation,
        koushi_state::SpaceMembersOperationState::Failed {
            request_id: failed_request_id,
            user_id: None,
            kind: OperationFailureKind::Sdk,
            ..
        } if failed_request_id == request_id
    ));
}

#[tokio::test]
async fn closed_account_forwarding_rolls_back_optimistic_space_invite() {
    let (state, failure, request_id) = run_closed_space_member_forwarding_case(
        SpaceMemberMembership::ChildRoomOnly,
        |request_id| koushi_protocol::command::RoomCommand::InviteUserToSpace {
            request_id,
            space_id: "!closed-forward-space:example.invalid".to_owned(),
            user_id: "@closed-forward-user:example.invalid".to_owned(),
            generation: 9,
        },
    )
    .await;

    assert_eq!(
        failure,
        CoreFailure::RoomOperationFailed {
            kind: RoomFailureKind::Sdk
        }
    );
    assert!(
        state
            .space_members
            .child_room_only
            .iter()
            .any(|entry| entry.user_id == "@closed-forward-user:example.invalid")
    );
    assert!(state.space_members.space_invited.is_empty());
    assert!(matches!(
        state.space_members.operation,
        koushi_state::SpaceMembersOperationState::Failed {
            request_id: failed_request_id,
            user_id: Some(ref failed_user_id),
            kind: OperationFailureKind::Sdk,
            ..
        } if failed_request_id == request_id
            && failed_user_id == "@closed-forward-user:example.invalid"
    ));
}

#[tokio::test]
async fn closed_account_forwarding_retains_invited_row_for_cancellation_retry() {
    let (state, failure, request_id) = run_closed_space_member_forwarding_case(
        SpaceMemberMembership::SpaceInvited,
        |request_id| koushi_protocol::command::RoomCommand::CancelSpaceInvite {
            request_id,
            space_id: "!closed-forward-space:example.invalid".to_owned(),
            user_id: "@closed-forward-user:example.invalid".to_owned(),
            generation: 9,
        },
    )
    .await;

    assert_eq!(
        failure,
        CoreFailure::RoomOperationFailed {
            kind: RoomFailureKind::Sdk
        }
    );
    assert!(
        state
            .space_members
            .space_invited
            .iter()
            .any(|entry| entry.user_id == "@closed-forward-user:example.invalid")
    );
    assert!(matches!(
        state.space_members.operation,
        koushi_state::SpaceMembersOperationState::Failed {
            request_id: failed_request_id,
            user_id: Some(ref failed_user_id),
            kind: OperationFailureKind::Sdk,
            ..
        } if failed_request_id == request_id
            && failed_user_id == "@closed-forward-user:example.invalid"
    ));
}

#[tokio::test]
async fn closed_account_forwarding_rolls_back_space_member_role_once() {
    let (state, failure, request_id) =
        run_closed_space_member_forwarding_case(SpaceMemberMembership::SpaceJoined, |request_id| {
            koushi_protocol::command::RoomCommand::UpdateSpaceMemberRole {
                request_id,
                space_id: "!closed-forward-space:example.invalid".to_owned(),
                user_id: "@closed-forward-user:example.invalid".to_owned(),
                generation: 9,
                expected_power_levels_revision: None,
                expected_power_level: 0,
                power_level: 50,
                confirmed: false,
            }
        })
        .await;

    assert_eq!(
        failure,
        CoreFailure::RoomOperationFailed {
            kind: RoomFailureKind::Sdk
        }
    );
    assert_eq!(
        state
            .space_members
            .space_joined
            .iter()
            .find(|entry| entry.user_id == "@closed-forward-user:example.invalid")
            .and_then(|entry| entry.power_level),
        Some(0)
    );
    assert!(matches!(
        state.space_members.operation,
        koushi_state::SpaceMembersOperationState::RoleUpdateFailed {
            request_id: failed_request_id,
            kind: koushi_state::SpaceMemberRoleFailureKind::Sdk,
            ..
        } if failed_request_id == request_id
    ));
}

pub(super) fn unread_diagnostic_room(room_id: &str) -> RoomSummary {
    RoomSummary {
        room_id: room_id.to_owned(),
        display_name: "Synthetic room".to_owned(),
        display_label: "Synthetic room".to_owned(),
        original_display_label: "Synthetic room".to_owned(),
        avatar: None,
        is_dm: false,
        dm_user_ids: Vec::new(),
        tags: RoomTags::default(),
        unread_count: 3,
        notification_count: 2,
        highlight_count: 1,
        marked_unread: true,
        recency_stamp: Some(42),
        conversation_activity: None,
        latest_event: None,
        parent_space_ids: Vec::new(),
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 2,
    }
}

#[test]
fn app_loop_trace_ignores_subthreshold_iterations() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let before = koushi_diagnostics::snapshot();
    app_loop_trace("test_boundary", 1, 2, Duration::from_millis(99));
    let after = koushi_diagnostics::snapshot();
    assert_eq!(
        after
            .records
            .iter()
            .filter(
                |record| record.event.source == "core.runtime" && record.event.stage == "app_loop"
            )
            .count(),
        before
            .records
            .iter()
            .filter(
                |record| record.event.source == "core.runtime" && record.event.stage == "app_loop"
            )
            .count()
    );
}

#[test]
fn app_loop_trace_records_at_threshold_without_environment_switch() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let before = koushi_diagnostics::snapshot();
    app_loop_trace("test_boundary", 3, 4, Duration::from_millis(100));
    let after = koushi_diagnostics::snapshot();
    assert!(after.records.len() > before.records.len());
    let record = after
        .records
        .iter()
        .rev()
        .find(|record| record.event.source == "core.runtime" && record.event.stage == "app_loop")
        .expect("threshold iteration should be collected");
    assert!(record.event.fields.iter().any(|field| field.key == "count"));
}

#[test]
fn default_data_dir_requires_home() {
    assert!(default_data_dir_from_home(None).is_err());
}

#[test]
fn default_data_dir_uses_xdg_like_user_data_path() {
    let dir = default_data_dir_from_home(Some("/tmp/synthetic-home".into())).unwrap();
    assert!(dir.ends_with(".local/share/koushi-desktop"));
}

#[test]
fn search_scope_round_trips_non_all_scope_kinds() {
    let scopes = [
        SearchScope::AllRooms,
        SearchScope::CurrentRoom {
            room_id: "!room:example.invalid".to_owned(),
        },
        SearchScope::CurrentSpace {
            space_id: "!space:example.invalid".to_owned(),
        },
    ];

    for scope in scopes {
        assert_eq!(
            map_state_search_scope_to_core(search_scope_to_state(&scope)),
            scope
        );
    }
}

#[tokio::test]
async fn versioned_snapshot_generation_matches_state_delta_generation() {
    let runtime = CoreRuntime::start_with_event_capacity(8);
    let mut connection = runtime.attach();

    runtime
        .inject_actions(vec![
            AppAction::AppStarted,
            AppAction::RestoreSessionSucceeded(SessionInfo {
                homeserver: "https://example.invalid".to_owned(),
                user_id: "@me:example.invalid".to_owned(),
                device_id: "DEVICE".to_owned(),
                authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
            }),
            AppAction::CurrentDeviceTrustChanged(koushi_state::CurrentDeviceTrustState::Verified),
        ])
        .await;

    let mut delta = None;
    for _ in 0..8 {
        let event =
            tokio::time::timeout(std::time::Duration::from_secs(1), connection.recv_event())
                .await
                .expect("runtime should emit state delta")
                .expect("event stream should stay open");
        if let CoreEvent::StateDelta(next) = event {
            delta = Some(next);
            break;
        }
    }
    let delta = delta.expect("expected state delta event");

    let snapshot = connection.versioned_snapshot();
    assert_eq!(snapshot.generation, delta.generation);
    assert_eq!(snapshot.generation, 1);
    assert!(matches!(
        snapshot.state.session,
        koushi_state::SessionState::Ready(_)
    ));
    runtime.shutdown_handle().abort();
}

#[tokio::test]
async fn rejected_space_invites_are_fenced_before_room_actor_route() {
    let runtime = CoreRuntime::start_with_event_capacity(64);
    let mut connection = runtime.attach();
    let space_id = "!space-a:example.invalid".to_owned();
    let duplicate_user_id = "@duplicate:example.invalid".to_owned();
    let generation = 7;

    runtime
        .inject_actions(vec![
            AppAction::AppStarted,
            AppAction::RestoreSessionSucceeded(SessionInfo {
                homeserver: "https://example.invalid".to_owned(),
                user_id: "@me:example.invalid".to_owned(),
                device_id: "DEVICE".to_owned(),
                authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
            }),
            AppAction::CurrentDeviceTrustChanged(koushi_state::CurrentDeviceTrustState::Verified),
            AppAction::SpaceMembersLoadRequested {
                request_id: 1,
                space_id: space_id.clone(),
                generation,
            },
            AppAction::SpaceMembersLoaded {
                request_id: 1,
                projection: SpaceMembersProjection {
                    space_id: space_id.clone(),
                    generation,
                    space_joined: Vec::new(),
                    space_invited: vec![SpaceMemberEntry {
                        user_id: duplicate_user_id.clone(),
                        display_name: None,
                        display_label: "Unknown user".to_owned(),
                        original_display_label: "Unknown user".to_owned(),
                        avatar_url: None,
                        power_level: None,
                        role: koushi_state::RoomMemberRole::User,
                        membership: SpaceMemberMembership::SpaceInvited,
                        child_room_ids: Vec::new(),
                        invite_pending: false,
                        role_options: Vec::new(),
                    }],
                    child_room_only: Vec::new(),
                    child_room_count: 0,
                    complete_child_room_count: 0,
                    incomplete_child_room_count: 0,
                    power_levels_revision: None,
                    can_edit_roles: false,
                },
            },
        ])
        .await;

    let expected_state = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let snapshot = connection.snapshot();
            if snapshot.space_members.selected_space_id.as_deref() == Some(space_id.as_str())
                && snapshot.space_members.generation == generation
                && snapshot.space_members.space_invited.len() == 1
            {
                break snapshot;
            }
            let _ = connection.recv_event().await.expect("runtime event stream");
        }
    })
    .await
    .expect("injected Space member state should settle");

    let rejected_commands = [
        (
            "wrong_space",
            "!space-b:example.invalid".to_owned(),
            generation,
        ),
        ("stale_generation", space_id.clone(), generation + 1),
        ("duplicate", space_id.clone(), generation),
    ];
    for (reason, target_space_id, target_generation) in rejected_commands {
        let request_id = connection.next_request_id();
        connection
            .command(CoreCommand::Room(
                koushi_protocol::command::RoomCommand::InviteUserToSpace {
                    request_id,
                    space_id: target_space_id,
                    user_id: duplicate_user_id.clone(),
                    generation: target_generation,
                },
            ))
            .await
            .expect("rejected invite command should enter the runtime");

        let failure = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                match connection.recv_event().await.expect("runtime event stream") {
                    CoreEvent::OperationFailed {
                        request_id: failed_request_id,
                        failure,
                    } if failed_request_id == request_id => break failure,
                    CoreEvent::Room(RoomEvent::SpaceMemberInviteSettled { .. }) => {
                        panic!("{reason} invite reached RoomActor settlement route")
                    }
                    _ => {}
                }
            }
        })
        .await
        .expect("rejected invite should emit a correlated failure");
        assert_eq!(
            failure,
            CoreFailure::RoomOperationFailed {
                kind: koushi_protocol::failure::RoomFailureKind::Sdk,
            }
        );
        assert_eq!(connection.snapshot(), expected_state);
    }

    let no_settlement = tokio::time::timeout(Duration::from_millis(100), async {
        loop {
            if let CoreEvent::Room(RoomEvent::SpaceMemberInviteSettled { .. }) =
                connection.recv_event().await.expect("runtime event stream")
            {
                return true;
            }
        }
    })
    .await;
    assert!(
        no_settlement.is_err(),
        "no rejected invite should reach the RoomActor/SDK settlement path"
    );
    runtime.shutdown_handle().abort();
}

#[tokio::test]
async fn projection_rejected_restore_emits_one_correlated_failure_without_routing() {
    let runtime = CoreRuntime::start_with_event_capacity(16);
    let mut connection = runtime.attach();
    runtime
        .inject_actions(vec![AppAction::LogoutRequested])
        .await;

    loop {
        let event = tokio::time::timeout(Duration::from_secs(1), connection.recv_event())
            .await
            .expect("logout projection should be published")
            .expect("event stream should remain open");
        if matches!(
            event,
            CoreEvent::StateDelta(delta)
                if matches!(delta.changed.session, Some(SessionState::LoggingOut))
        ) {
            break;
        }
    }

    let restore_request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Account(AccountCommand::RestoreSession {
            request_id: restore_request_id,
            account_key: AccountKey("@restore-rejected:example.invalid".to_owned()),
        }))
        .await
        .expect("restore command should enter the bounded runtime inbox");
    let marker_request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Account(AccountCommand::QuerySavedSessions {
            request_id: marker_request_id,
        }))
        .await
        .expect("ordered marker should enter the bounded runtime inbox");

    let mut restore_failure_count = 0;
    loop {
        let event = tokio::time::timeout(Duration::from_secs(1), connection.recv_event())
            .await
            .expect("projection rejection should settle before the ordered marker")
            .expect("event stream should remain open");
        match event {
            CoreEvent::OperationFailed {
                request_id,
                failure: CoreFailure::SessionRequired,
            } if request_id == restore_request_id => {
                restore_failure_count += 1;
            }
            CoreEvent::OperationFailed { request_id, .. } if request_id == restore_request_id => {
                panic!("projection rejection emitted the wrong failure kind")
            }
            CoreEvent::Account(AccountEvent::SessionRestored { request_id, .. })
                if request_id == restore_request_id =>
            {
                panic!("projection-rejected restore was routed to AccountActor")
            }
            CoreEvent::Account(AccountEvent::SavedSessionsListed { request_id, .. })
                if request_id == marker_request_id =>
            {
                break;
            }
            _ => {}
        }
    }

    assert_eq!(
        restore_failure_count, 1,
        "a projection-rejected command must have exactly one terminal failure"
    );
    assert!(matches!(
        connection.snapshot().session,
        SessionState::LoggingOut
    ));
    runtime.shutdown_handle().abort();
}

#[tokio::test]
async fn actor_profile_changes_emit_timeline_display_label_updates() {
    let runtime = CoreRuntime::start_with_event_capacity(8);
    let mut connection = runtime.attach();

    runtime
        .inject_actions(vec![
            AppAction::AppStarted,
            AppAction::RestoreSessionSucceeded(SessionInfo {
                homeserver: "https://example.invalid".to_owned(),
                user_id: "@me:example.invalid".to_owned(),
                device_id: "DEVICE".to_owned(),
                authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
            }),
            AppAction::CurrentDeviceTrustChanged(koushi_state::CurrentDeviceTrustState::Verified),
            AppAction::UserProfilesUpdated {
                profiles: vec![UserProfile {
                    user_id: "@alice:example.invalid".to_owned(),
                    display_name: Some("Alice Upstream".to_owned()),
                    display_label: String::new(),
                    original_display_label: String::new(),
                    mention_search_terms: Vec::new(),
                    avatar: None,
                }],
            },
            AppAction::LocalUserAliasesLoaded {
                aliases: BTreeMap::from([(
                    "@alice:example.invalid".to_owned(),
                    "Alice Alias".to_owned(),
                )]),
            },
        ])
        .await;

    let mut saw_alias_update = false;
    for _ in 0..4 {
        let event =
            tokio::time::timeout(std::time::Duration::from_secs(1), connection.recv_event())
                .await
                .expect("runtime should emit profile/timeline events")
                .expect("event stream should stay open");
        if let CoreEvent::Timeline(TimelineEvent::DisplayLabelsUpdated { labels }) = event
            && labels.iter().any(|label| {
                label.user_id == "@alice:example.invalid" && label.display_label == "Alice Alias"
            })
        {
            saw_alias_update = true;
            break;
        }
    }

    assert!(
        saw_alias_update,
        "actor-origin ProfileChanged effects must relabel already-loaded timeline rows"
    );
    runtime.shutdown_handle().abort();
}

#[tokio::test]
async fn settings_update_emits_timeline_display_policy_update() {
    let runtime = CoreRuntime::start_with_event_capacity(16);
    let mut connection = runtime.attach();

    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::App(
            koushi_protocol::command::AppCommand::UpdateSettings {
                request_id,
                patch: SettingsPatch {
                    display: Some(DisplaySettings {
                        code_block_wrap: true,
                        hide_redacted: true,
                        url_previews_enabled: true,
                        encrypted_url_previews_enabled: false,
                    }),
                    ..SettingsPatch::default()
                },
            },
        ))
        .await
        .expect("settings update command should be accepted");

    let mut saw_policy_update = false;
    for _ in 0..4 {
        let event =
            tokio::time::timeout(std::time::Duration::from_secs(1), connection.recv_event())
                .await
                .expect("runtime should emit settings/timeline events")
                .expect("event stream should stay open");
        if let CoreEvent::Timeline(TimelineEvent::DisplayPolicyUpdated { hide_redacted }) = event {
            saw_policy_update = hide_redacted;
            break;
        }
    }

    assert!(
        saw_policy_update,
        "SettingsChanged must reproject already-loaded redacted timeline rows"
    );
    runtime.shutdown_handle().abort();
}

#[tokio::test]
async fn local_alias_clear_command_emits_target_display_label_update() {
    let runtime = CoreRuntime::start_with_event_capacity(16);
    let mut connection = runtime.attach();
    let user_id = "@unknown:example.invalid";

    runtime
        .inject_actions(vec![
            AppAction::AppStarted,
            AppAction::RestoreSessionSucceeded(SessionInfo {
                homeserver: "https://example.invalid".to_owned(),
                user_id: "@me:example.invalid".to_owned(),
                device_id: "DEVICE".to_owned(),
                authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
            }),
            AppAction::CurrentDeviceTrustChanged(koushi_state::CurrentDeviceTrustState::Verified),
            AppAction::LocalUserAliasesLoaded {
                aliases: BTreeMap::from([(user_id.to_owned(), "Unknown Alias".to_owned())]),
            },
        ])
        .await;

    for _ in 0..4 {
        let event =
            tokio::time::timeout(std::time::Duration::from_secs(1), connection.recv_event())
                .await
                .expect("runtime should emit initial profile events")
                .expect("event stream should stay open");
        if matches!(event, CoreEvent::StateDelta(_)) {
            break;
        }
    }

    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Account(AccountCommand::SetLocalUserAlias {
            request_id,
            user_id: user_id.to_owned(),
            alias: None,
        }))
        .await
        .expect("alias clear command should be accepted");

    let mut saw_clear_update = false;
    for _ in 0..4 {
        let event =
            tokio::time::timeout(std::time::Duration::from_secs(1), connection.recv_event())
                .await
                .expect("runtime should emit alias-clear events")
                .expect("event stream should stay open");
        if let CoreEvent::Timeline(TimelineEvent::DisplayLabelsUpdated { labels }) = event
            && labels
                .iter()
                .any(|label| label.user_id == user_id && label.display_label == user_id)
        {
            saw_clear_update = true;
            break;
        }
    }

    assert!(
        saw_clear_update,
        "alias clear must relabel rows even when the target user is absent from profile.users"
    );
    runtime.shutdown_handle().abort();
}

#[test]
fn current_session_status_account_command_projects_open_and_manual_refreshes() {
    for trigger in [
        koushi_state::SessionStatusRefreshTrigger::Open,
        koushi_state::SessionStatusRefreshTrigger::Manual,
    ] {
        assert_eq!(
            account_command_projected_action(&AccountCommand::RefreshCurrentSessionStatus {
                request_id: RequestId {
                    connection_id: RuntimeConnectionId(2),
                    sequence: 17,
                },
                trigger,
            }),
            Some(AppAction::CurrentSessionStatusRefreshRequested {
                request_id: 17,
                trigger,
            })
        );
    }
}

#[test]
fn current_session_status_duplicate_has_a_full_request_id_correlated_benign_noop() {
    let request_id = RequestId {
        connection_id: RuntimeConnectionId(9),
        sequence: 42,
    };
    assert!(current_session_status_noop_event(request_id, false, 7).is_none());
    assert!(matches!(
        current_session_status_noop_event(request_id, true, 7),
        Some(CoreEvent::IntentLifecycle {
            request_id: event_request_id,
            outcome: IntentOutcome::BenignNoOp(IntentNoOpReason::AlreadyActive),
            published_generation: 7,
        }) if event_request_id == request_id
    ));
}

#[test]
fn replacement_thread_helper_preserves_same_key_and_unsubscribes_different_key() {
    let account_key = AccountKey("@alice:example.invalid".to_owned());
    let current = TimelineKey {
        account_key: account_key.clone(),
        kind: TimelineKind::Thread {
            room_id: "!room:example.invalid".to_owned(),
            root_event_id: "$root-a:example.invalid".to_owned(),
        },
    };
    let same = current.clone();
    let different = TimelineKey {
        account_key,
        kind: TimelineKind::Thread {
            room_id: "!room:example.invalid".to_owned(),
            root_event_id: "$root-b:example.invalid".to_owned(),
        },
    };

    assert_eq!(
        unsubscribe_replaced_thread_timeline_key(Some(current.clone()), same),
        None
    );
    assert_eq!(
        unsubscribe_replaced_thread_timeline_key(Some(current.clone()), different),
        Some(current)
    );
    assert_eq!(
        unsubscribe_replaced_thread_timeline_key(None, thread_key("$root-c:example.invalid")),
        None
    );
}

#[tokio::test]
async fn committed_room_cleanup_bypasses_a_saturated_account_mailbox() {
    let data_dir = tempfile::tempdir().expect("runtime data directory");
    let (account_tx, mut saturated_account_rx) = mpsc::channel(1);
    account_tx
        .try_send(AccountMessage::CancelActivityResolution)
        .expect("fill the ordinary AccountActor mailbox");
    let (navigation_projection, navigation_projection_rx) =
        crate::timeline::NavigationProjectionIngress::channel();
    drop(navigation_projection_rx);
    let account_actor =
        AccountActorHandle::for_app_actor_test(account_tx, navigation_projection.clone());

    let session = SessionInfo {
        homeserver: "https://example.invalid".to_owned(),
        user_id: "@synthetic:example.invalid".to_owned(),
        device_id: "SYNTHETIC".to_owned(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    };
    let session_key = session_key_id_from_info(&session);
    let old_room = "!old:example.invalid";
    let next_room = "!next:example.invalid";
    let mut state = AppState {
        session: SessionState::Ready(session),
        rooms: vec![
            unread_diagnostic_room(old_room),
            unread_diagnostic_room(next_room),
        ],
        ..AppState::default()
    };
    state.navigation.active_room_id = Some(old_room.to_owned());

    let (command_tx, command_rx) = mpsc::channel(1);
    let (action_tx, action_rx) = mpsc::channel(1);
    let (_composer_draft_test_tx, composer_draft_test_rx) = mpsc::channel(1);
    let (event_tx, mut event_rx) = broadcast::channel(16);
    let (snapshot_tx, mut snapshot_rx) = watch::channel(VersionedAppStateSnapshot {
        generation: 0,
        state: state.clone(),
    });
    let request_id = RequestId {
        connection_id: RuntimeConnectionId(91),
        sequence: 7,
    };
    let mut pending_select = HashMap::new();
    pending_select.insert(
        next_room.to_owned(),
        std::collections::VecDeque::from([request_id]),
    );
    let composer_draft_leases = Arc::new(ComposerDraftLeaseRegistry::new());
    let composer_draft_lease_changes = composer_draft_leases.subscribe();
    let (composer_draft_rejected_tx, composer_draft_rejected_rx) = mpsc::unbounded_channel();
    let (_focused_projection_tx, focused_projection_rx) = mpsc::unbounded_channel();
    let actor = AppActor {
        command_rx,
        action_rx,
        focused_projection_rx: Some(focused_projection_rx),
        composer_draft_test_rx,
        event_tx,
        snapshot_tx,
        state,
        settings_store: SettingsStore::new(data_dir.path()),
        settings_load_status: SettingsLoadStatus::Loaded,
        composer_draft_store_actor: StoreActor::new(data_dir.path().to_owned()),
        composer_draft_load_status: ComposerDraftLoadStatus::Loaded(session_key.clone()),
        composer_draft_reload_required: false,
        navigation_loaded_for: Some(session_key.clone()),
        navigation_persistence_status: NavigationPersistenceStatus::Loaded(session_key.clone()),
        scheduled_sends_loaded_for: Some(session_key.clone()),
        room_preferences_loaded_for: Some(session_key),
        state_generation: 0,
        pending_composer_draft_persist: None,
        composer_draft_leases,
        composer_draft_lease_changes,
        composer_draft_rejected_tx,
        composer_draft_rejected_rx,
        pending_composer_acceptances: HashMap::new(),
        pending_command_admissions: Vec::new(),
        account_actor,
        activity_projection: ActivityProjection::default(),
        activity_resolution_generation: 0,
        next_internal_request_sequence: 1,
        navigation_projection_generation: 0,
        pending_select,
        pending_focused_navigation: None,
        latest_focused_projection_generation: HashMap::new(),
        pending_date_navigation_request_id: None,
    };
    let actor_task = executor::spawn(actor.run());

    action_tx
        .send(vec![AppAction::SelectRoom {
            room_id: next_room.to_owned(),
        }])
        .await
        .expect("inject committed room selection");

    let terminal = executor::timeout(Duration::from_secs(1), async {
        let published = event_rx.recv().await.expect("event stream remains open");
        assert!(matches!(&published, CoreEvent::StateDelta(delta) if delta.generation == 1));
        loop {
            match event_rx.recv().await.expect("event stream remains open") {
                event @ CoreEvent::IntentLifecycle { .. } => break event,
                _ => {}
            }
        }
    })
    .await
    .expect("publication and terminal must not wait for cleanup transport");
    assert!(matches!(
        terminal,
        CoreEvent::IntentLifecycle {
            request_id: observed,
            outcome: IntentOutcome::Committed,
            published_generation: 1,
        } if observed == request_id
    ));
    assert!(matches!(
        event_rx.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));
    assert_eq!(
        snapshot_rx
            .borrow_and_update()
            .state
            .navigation
            .active_room_id
            .as_deref(),
        Some(next_room)
    );
    let mut retained_rx = navigation_projection.subscribe();
    let retained = retained_rx
        .borrow_and_update()
        .clone()
        .expect("cleanup and replacement projection remain latest-wins");
    assert_eq!(
        retained.cleanup.cancel_pagination,
        Some(TimelineKey::room(
            AccountKey("@synthetic:example.invalid".to_owned()),
            old_room,
        ))
    );
    assert_eq!(
        retained.cleanup.cancel_link_previews,
        retained.cleanup.cancel_pagination
    );

    actor_task.abort();
    drop(command_tx);
    drop(action_tx);
    assert!(
        saturated_account_rx.try_recv().is_ok(),
        "ordinary mailbox remained saturated throughout the selection"
    );
}

#[tokio::test]
async fn same_batch_select_room_settles_only_final_selection() {
    let data_dir = tempfile::tempdir().expect("runtime data directory");
    let (account_tx, _account_rx) = mpsc::channel(16);
    let (navigation_projection, navigation_projection_rx) =
        crate::timeline::NavigationProjectionIngress::channel();
    drop(navigation_projection_rx);
    let account_actor =
        AccountActorHandle::for_app_actor_test(account_tx, navigation_projection.clone());

    let session = SessionInfo {
        homeserver: "https://example.invalid".to_owned(),
        user_id: "@synthetic:example.invalid".to_owned(),
        device_id: "SYNTHETIC".to_owned(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    };
    let session_key = session_key_id_from_info(&session);
    let first_room = "!first:example.invalid";
    let second_room = "!second:example.invalid";
    let mut state = AppState {
        session: SessionState::Ready(session),
        rooms: vec![
            unread_diagnostic_room(first_room),
            unread_diagnostic_room(second_room),
        ],
        ..AppState::default()
    };
    // Exercise the defensive case where the first selection was already
    // active at reduce time but is replaced before this batch publishes.
    state.navigation.active_room_id = Some(first_room.to_owned());

    let (command_tx, command_rx) = mpsc::channel(1);
    let (action_tx, action_rx) = mpsc::channel(1);
    let (_composer_draft_test_tx, composer_draft_test_rx) = mpsc::channel(1);
    let (event_tx, mut event_rx) = broadcast::channel(16);
    let (snapshot_tx, mut snapshot_rx) = watch::channel(VersionedAppStateSnapshot {
        generation: 0,
        state: state.clone(),
    });
    let first_request = RequestId {
        connection_id: RuntimeConnectionId(92),
        sequence: 1,
    };
    let second_request = RequestId {
        connection_id: RuntimeConnectionId(92),
        sequence: 2,
    };
    let already_active_request = RequestId {
        connection_id: RuntimeConnectionId(92),
        sequence: 3,
    };
    let mut pending_select = HashMap::new();
    pending_select.insert(
        first_room.to_owned(),
        std::collections::VecDeque::from([first_request]),
    );
    pending_select.insert(
        second_room.to_owned(),
        std::collections::VecDeque::from([second_request, already_active_request]),
    );
    let composer_draft_leases = Arc::new(ComposerDraftLeaseRegistry::new());
    let composer_draft_lease_changes = composer_draft_leases.subscribe();
    let (composer_draft_rejected_tx, composer_draft_rejected_rx) = mpsc::unbounded_channel();
    let (_focused_projection_tx, focused_projection_rx) = mpsc::unbounded_channel();
    let actor = AppActor {
        command_rx,
        action_rx,
        focused_projection_rx: Some(focused_projection_rx),
        composer_draft_test_rx,
        event_tx,
        snapshot_tx,
        state,
        settings_store: SettingsStore::new(data_dir.path()),
        settings_load_status: SettingsLoadStatus::Loaded,
        composer_draft_store_actor: StoreActor::new(data_dir.path().to_owned()),
        composer_draft_load_status: ComposerDraftLoadStatus::Loaded(session_key.clone()),
        composer_draft_reload_required: false,
        navigation_loaded_for: Some(session_key.clone()),
        navigation_persistence_status: NavigationPersistenceStatus::Loaded(session_key.clone()),
        scheduled_sends_loaded_for: Some(session_key.clone()),
        room_preferences_loaded_for: Some(session_key),
        state_generation: 0,
        pending_composer_draft_persist: None,
        composer_draft_leases,
        composer_draft_lease_changes,
        composer_draft_rejected_tx,
        composer_draft_rejected_rx,
        pending_composer_acceptances: HashMap::new(),
        pending_command_admissions: Vec::new(),
        account_actor,
        activity_projection: ActivityProjection::default(),
        activity_resolution_generation: 0,
        next_internal_request_sequence: 1,
        navigation_projection_generation: 0,
        pending_select,
        pending_focused_navigation: None,
        latest_focused_projection_generation: HashMap::new(),
        pending_date_navigation_request_id: None,
    };
    let actor_task = executor::spawn(actor.run());

    action_tx
        .send(vec![
            AppAction::SelectRoom {
                room_id: first_room.to_owned(),
            },
            AppAction::SelectRoom {
                room_id: second_room.to_owned(),
            },
        ])
        .await
        .expect("inject same-batch room selections");

    let outcomes = executor::timeout(Duration::from_secs(1), async {
        let mut outcomes = Vec::new();
        while outcomes.len() < 2 {
            if let CoreEvent::IntentLifecycle {
                request_id,
                outcome,
                ..
            } = event_rx.recv().await.expect("event stream remains open")
            {
                outcomes.push((request_id, outcome));
            }
        }
        outcomes
    })
    .await
    .expect("both selections must settle");
    assert_eq!(
        outcomes,
        vec![
            (
                first_request,
                IntentOutcome::FailedNoOp(IntentNoOpReason::Superseded),
            ),
            (second_request, IntentOutcome::Committed),
        ]
    );

    snapshot_rx
        .changed()
        .await
        .expect("snapshot channel remains open");
    assert_eq!(
        snapshot_rx
            .borrow()
            .state
            .navigation
            .active_room_id
            .as_deref(),
        Some(second_room)
    );
    assert_eq!(snapshot_rx.borrow().generation, 1);

    action_tx
        .send(vec![AppAction::SelectRoom {
            room_id: second_room.to_owned(),
        }])
        .await
        .expect("inject already-active selection");
    let no_op = executor::timeout(Duration::from_secs(1), event_rx.recv())
        .await
        .expect("already-active intent must settle")
        .expect("event stream remains open");
    assert!(matches!(
        no_op,
        CoreEvent::IntentLifecycle {
            request_id,
            outcome: IntentOutcome::BenignNoOp(IntentNoOpReason::AlreadyActive),
            published_generation: 1,
        } if request_id == already_active_request
    ));
    assert_eq!(
        snapshot_rx.borrow().generation,
        1,
        "a no-op settlement must not fabricate a new StateDelta"
    );

    actor_task.abort();
    drop(command_tx);
    drop(action_tx);
}

#[test]
fn bootstrap_cross_signing_command_projects_pending_state_before_account_route() {
    let request_id = RequestId {
        connection_id: RuntimeConnectionId(1),
        sequence: 6,
    };
    assert_eq!(
        account_command_projected_action(&AccountCommand::BootstrapCrossSigning {
            request_id,
            auth: None,
        }),
        Some(AppAction::BootstrapCrossSigningRequested { request_id: 6 })
    );
}

#[test]
fn identity_reset_auth_command_projects_pending_state_before_routing() {
    let request_id = RequestId {
        connection_id: RuntimeConnectionId(1),
        sequence: 7,
    };
    let flow_id = 99;

    assert_eq!(
        account_command_projected_action(&AccountCommand::SubmitIdentityResetAuth {
            request_id,
            flow_id,
            request: koushi_state::IdentityResetAuthRequest::OAuthApproved,
        }),
        Some(AppAction::ResetIdentityAuthSubmitted {
            request_id: flow_id
        })
    );
}

#[test]
fn oidc_completion_has_no_speculative_appactor_projection() {
    let request_id = RequestId {
        connection_id: RuntimeConnectionId(1),
        sequence: 8,
    };

    assert_eq!(
        account_command_projected_action(&AccountCommand::CompleteOidcLogin {
            request_id,
            callback_url: "koushi-desktop://auth/callback?code=secret".to_owned(),
            platform: koushi_state::DisplayPlatform::Linux,
        }),
        None
    );
}

#[test]
fn change_homeserver_has_no_speculative_app_projection() {
    let command = AccountCommand::ChangeHomeserver {
        request_id: RequestId {
            connection_id: RuntimeConnectionId(4),
            sequence: 12,
        },
    };

    assert_eq!(account_command_projected_action(&command), None);
}

#[test]
fn oidc_authorization_start_only_projects_discovery() {
    let request_id = RequestId {
        connection_id: RuntimeConnectionId(1),
        sequence: 7,
    };

    assert_eq!(
        account_command_projected_action(&AccountCommand::StartOidcLogin {
            request_id,
            homeserver: "https://matrix.example.org".to_owned(),
        }),
        Some(AppAction::LoginDiscoveryRequested {
            homeserver: "https://matrix.example.org".to_owned(),
        })
    );
}

#[test]
fn restore_key_backup_command_projects_state_without_recovery_secret() {
    let request_id = RequestId {
        connection_id: RuntimeConnectionId(1),
        sequence: 9,
    };

    assert_eq!(
        account_command_projected_action(&AccountCommand::RestoreKeyBackup {
            request_id,
            version: Some("backup-version-1".to_owned()),
            request: koushi_state::RecoveryRequest {
                secret: koushi_state::AuthSecret::new("recovery secret"),
            },
        }),
        Some(AppAction::RestoreKeyBackupRequested {
            request_id: 9,
            version: Some("backup-version-1".to_owned()),
        })
    );
}

#[test]
fn reset_local_data_command_projects_resetting_state_before_routing() {
    let request_id = RequestId {
        connection_id: RuntimeConnectionId(1),
        sequence: 17,
    };

    assert_eq!(
        account_command_projected_action(&AccountCommand::ResetLocalData { request_id }),
        Some(AppAction::ResetLocalDataRequested { request_id: 17 })
    );
}

#[test]
fn device_cleanup_commands_project_correlated_pending_state_before_routing() {
    let start_request_id = RequestId {
        connection_id: RuntimeConnectionId(1),
        sequence: 21,
    };
    let submit_request_id = RequestId {
        connection_id: RuntimeConnectionId(1),
        sequence: 22,
    };

    assert_eq!(
        account_command_projected_action(&AccountCommand::StartDeviceCleanup {
            request_id: start_request_id,
        }),
        Some(AppAction::DeviceCleanupStartRequested { request_id: 21 })
    );
    assert_eq!(
        account_command_projected_action(&AccountCommand::SubmitDeviceCleanupUia {
            request_id: submit_request_id,
            flow_id: 21,
            password: koushi_state::AuthSecret::new("private-password"),
        }),
        Some(AppAction::DeviceCleanupUiaSubmitted {
            request_id: 21,
            flow_id: 21,
        })
    );
    assert_eq!(
        account_command_projected_action(&AccountCommand::EraseDeviceCleanupLocalDataAnyway {
            request_id: submit_request_id,
        },),
        Some(AppAction::DeviceCleanupEraseLocalAnywayRequested { request_id: 22 })
    );
}

#[test]
fn device_cleanup_commands_are_admitted_from_the_provisional_gate() {
    let command = CoreCommand::Account(AccountCommand::StartDeviceCleanup {
        request_id: RequestId {
            connection_id: RuntimeConnectionId(1),
            sequence: 23,
        },
    });
    let session = SessionState::AwaitingVerification {
        info: koushi_state::SessionInfo {
            homeserver: "https://example.invalid".to_owned(),
            user_id: "@user:example.invalid".to_owned(),
            device_id: "DEVICE".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
        gate: koushi_state::VerificationGateState {
            methods: vec![],
            account_kind: koushi_state::VerificationAccountKind::ExistingIdentity,
            failure: Some(koushi_state::VerificationGateFailureKind::Sdk),
        },
    };

    assert!(is_verification_gate_command(&command, &session));
}

#[test]
fn profile_commands_project_pending_state_without_display_name_or_avatar_bytes() {
    let display_request_id = RequestId {
        connection_id: RuntimeConnectionId(1),
        sequence: 13,
    };
    let avatar_request_id = RequestId {
        connection_id: RuntimeConnectionId(1),
        sequence: 14,
    };

    assert_eq!(
        account_command_projected_action(&AccountCommand::SetDisplayName {
            request_id: display_request_id,
            display_name: Some("Private Display".to_owned()),
        }),
        Some(AppAction::ProfileUpdateRequested {
            request_id: 13,
            request: ProfileUpdateRequest::SetDisplayName {
                display_name: Some("Private Display".to_owned()),
            },
        })
    );

    assert_eq!(
        account_command_projected_action(&AccountCommand::SetAvatar {
            request_id: avatar_request_id,
            request: koushi_protocol::command::SetAvatarRequest {
                mime_type: "image/png".to_owned(),
                bytes: vec![1, 2, 3, 4],
            },
        }),
        Some(AppAction::ProfileUpdateRequested {
            request_id: 14,
            request: ProfileUpdateRequest::SetAvatar {
                mime_type: "image/png".to_owned(),
                byte_count: 4,
            },
        })
    );
}

#[test]
fn local_user_alias_command_projects_pending_state_without_leaking_alias() {
    let request_id = RequestId {
        connection_id: RuntimeConnectionId(1),
        sequence: 15,
    };

    assert_eq!(
        account_command_projected_action(&AccountCommand::SetLocalUserAlias {
            request_id,
            user_id: "@private:example.invalid".to_owned(),
            alias: Some("Private Alias".to_owned()),
        }),
        Some(AppAction::LocalUserAliasUpdateRequested {
            request_id: 15,
            user_id: "@private:example.invalid".to_owned(),
            alias: Some("Private Alias".to_owned()),
        })
    );
}

#[test]
fn verification_followup_commands_project_flow_id_without_speculative_cancel() {
    let request_id = RequestId {
        connection_id: RuntimeConnectionId(1),
        sequence: 9,
    };
    let flow_id = 42;

    assert_eq!(
        account_command_projected_action(&AccountCommand::AcceptVerification {
            request_id,
            flow_id,
        }),
        Some(AppAction::VerificationAccepted {
            request_id: flow_id,
        })
    );
    assert_eq!(
        account_command_projected_action(&AccountCommand::ConfirmSasVerification {
            request_id,
            flow_id,
        }),
        Some(AppAction::VerificationConfirmed {
            request_id: flow_id,
        })
    );
    assert_eq!(
        account_command_projected_action(&AccountCommand::CancelVerification {
            request_id,
            flow_id,
            reason: koushi_state::VerificationCancelReason::User,
        }),
        None
    );
}

#[test]
fn trust_discovery_retry_is_admitted_only_in_retryable_gate_states() {
    let command = CoreCommand::Account(AccountCommand::RetryCurrentDeviceTrustDiscovery {
        request_id: RequestId {
            connection_id: RuntimeConnectionId(1),
            sequence: 77,
        },
    });
    let info = SessionInfo {
        homeserver: "https://example.invalid".into(),
        user_id: "@me:example.invalid".into(),
        device_id: "DEVICE".into(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    };
    let gate = koushi_state::VerificationGateState {
        methods: vec![],
        account_kind: koushi_state::VerificationAccountKind::ExistingIdentity,
        failure: Some(koushi_state::VerificationGateFailureKind::Network),
    };
    assert!(is_verification_gate_command(
        &command,
        &SessionState::Provisional {
            info: info.clone(),
            phase: koushi_state::ProvisionalPhase::RecheckingTrust {
                failure: Some(koushi_state::VerificationGateFailureKind::Network)
            }
        }
    ));
    assert!(is_verification_gate_command(
        &command,
        &SessionState::AwaitingVerification {
            info: info.clone(),
            gate: gate.clone()
        }
    ));
    assert!(!is_verification_gate_command(
        &command,
        &SessionState::Verifying {
            info,
            gate,
            method: koushi_state::VerificationMethod::RecoveryKey,
            flow_id: 77,
            sas_emojis: vec![]
        }
    ));
}

#[test]
fn local_data_reset_is_admitted_through_the_verification_gate() {
    let command = CoreCommand::Account(AccountCommand::ResetLocalData {
        request_id: RequestId {
            connection_id: RuntimeConnectionId(1),
            sequence: 78,
        },
    });
    let info = SessionInfo {
        homeserver: "https://example.invalid".into(),
        user_id: "@me:example.invalid".into(),
        device_id: "DEVICE".into(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    };
    let gate = koushi_state::VerificationGateState {
        methods: vec![],
        account_kind: koushi_state::VerificationAccountKind::ExistingIdentity,
        failure: Some(koushi_state::VerificationGateFailureKind::Sdk),
    };

    assert!(is_verification_gate_command(
        &command,
        &SessionState::Provisional {
            info: info.clone(),
            phase: koushi_state::ProvisionalPhase::DiscoveringMethods,
        }
    ));
    assert!(is_verification_gate_command(
        &command,
        &SessionState::AwaitingVerification {
            info: info.clone(),
            gate: gate.clone(),
        }
    ));
    assert!(is_verification_gate_command(
        &command,
        &SessionState::Verifying {
            info,
            gate,
            method: koushi_state::VerificationMethod::RecoveryKey,
            flow_id: 78,
            sas_emojis: vec![],
        }
    ));
    assert!(!is_verification_gate_command(
        &command,
        &SessionState::SignedOut
    ));
}

#[test]
fn device_cleanup_is_not_admitted_while_verification_owns_the_gate() {
    let command = CoreCommand::Account(AccountCommand::StartDeviceCleanup {
        request_id: RequestId {
            connection_id: RuntimeConnectionId(1),
            sequence: 79,
        },
    });
    let info = SessionInfo {
        homeserver: "https://example.invalid".into(),
        user_id: "@me:example.invalid".into(),
        device_id: "DEVICE".into(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    };
    let gate = koushi_state::VerificationGateState {
        methods: vec![koushi_state::VerificationMethodCapability::RecoveryKey],
        account_kind: koushi_state::VerificationAccountKind::ExistingIdentity,
        failure: Some(koushi_state::VerificationGateFailureKind::Sdk),
    };

    assert!(is_verification_gate_command(
        &command,
        &SessionState::AwaitingVerification {
            info: info.clone(),
            gate: gate.clone(),
        }
    ));
    assert!(!is_verification_gate_command(
        &command,
        &SessionState::Verifying {
            info,
            gate,
            method: koushi_state::VerificationMethod::RecoveryKey,
            flow_id: 79,
            sas_emojis: vec![],
        }
    ));
}

#[test]
fn gate_sas_and_bootstrap_commands_project_only_opaque_flow_state() {
    let request_id = RequestId {
        connection_id: RuntimeConnectionId(5),
        sequence: 90,
    };
    assert_eq!(
        account_command_projected_action(&AccountCommand::StartOwnUserSas {
            request_id,
            flow_id: 31,
        }),
        Some(AppAction::VerificationMethodSubmitted {
            method: koushi_state::VerificationMethod::ExistingDeviceSas,
            flow_id: 31,
        })
    );
    assert_eq!(
        account_command_projected_action(&AccountCommand::ConfirmSessionBootstrapSaved {
            request_id,
            flow_id: 32,
        }),
        Some(AppAction::BootstrapRecoverySavedConfirmed { flow_id: 32 })
    );
    let debug = format!(
        "{:?}",
        AccountCommand::StartOwnUserSas {
            request_id,
            flow_id: 31,
        }
    );
    assert!(!debug.contains('@'));
    assert!(!debug.contains("DEVICE"));
    let bootstrap_debug = format!(
        "{:?}",
        AccountCommand::StartSessionBootstrap {
            request_id,
            flow_id: 32,
            auth: Some(koushi_state::AuthSecret::new("private-auth")),
            request: koushi_protocol::command::SecureBackupSetupRequest {
                passphrase: Some(koushi_state::AuthSecret::new("private-passphrase")),
                recovery_key_destination_requested: true,
                intent: koushi_state::SecureBackupSetupIntent::InitialSetup,
            },
        }
    );
    for forbidden in ["private-auth", "private-passphrase", "/private/"] {
        assert!(!bootstrap_debug.contains(forbidden));
    }
}

fn thread_key(root_event_id: &str) -> TimelineKey {
    TimelineKey {
        account_key: AccountKey("@alice:example.invalid".to_owned()),
        kind: TimelineKind::Thread {
            room_id: "!room:example.invalid".to_owned(),
            root_event_id: root_event_id.to_owned(),
        },
    }
}

#[tokio::test]
async fn authoritative_trust_runs_through_app_actor_ack_and_restarts_real_children() {
    use std::io::{Read, Write};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let homeserver = format!("http://{}", listener.local_addr().expect("address"));
    std::thread::spawn(move || {
        for _ in 0..4096 {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            std::thread::spawn(move || {
                let mut request = Vec::new();
                let mut buffer = [0_u8; 4096];
                loop {
                    let count = stream.read(&mut buffer).expect("read");
                    request.extend_from_slice(&buffer[..count]);
                    let text = String::from_utf8_lossy(&request);
                    let Some(end) = text.find("\r\n\r\n") else {
                        continue;
                    };
                    let length = text
                        .lines()
                        .find_map(|line| line.strip_prefix("Content-Length: "))
                        .and_then(|value| value.parse::<usize>().ok())
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
                let text = String::from_utf8_lossy(&request);
                let body = if text.starts_with("GET /_matrix/client/versions ") {
                    r#"{"versions":["v1.7"],"unstable_features":{"org.matrix.simplified_msc3575":true}}"#
                } else if text.contains("/_matrix/client/") && text.contains("login") {
                    let requested_device_id = text
                        .split_once("\r\n\r\n")
                        .and_then(|(_, body)| serde_json::from_str::<serde_json::Value>(body).ok())
                        .and_then(|body| body["device_id"].as_str().map(str::to_owned))
                        .unwrap_or_else(|| "FIXTUREDEVICE".to_owned());
                    let body = format!(
                        r#"{{"access_token":"fixture-token","device_id":"{requested_device_id}","user_id":"@fixture-user:example.invalid"}}"#
                    );
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream.write_all(response.as_bytes()).expect("write");
                    return;
                } else if text
                    .contains("/_matrix/client/unstable/org.matrix.simplified_msc3575/sync")
                {
                    if text.contains("\"conn_id\":\"room-list\"") {
                        r#"{"pos":"sliding-pos","lists":{"all_rooms":{"count":1,"ops":[{"op":"SYNC","range":[0,0],"room_ids":["!fixture-room:example.invalid"]}]}},"rooms":{"!fixture-room:example.invalid":{"initial":true,"required_state":[{"type":"m.room.create","state_key":"","sender":"@fixture-user:example.invalid","event_id":"$create:example.invalid","origin_server_ts":1,"content":{"creator":"@fixture-user:example.invalid","room_version":"10"}},{"type":"m.room.name","state_key":"","sender":"@fixture-user:example.invalid","event_id":"$name:example.invalid","origin_server_ts":2,"content":{"name":"Fixture room"}},{"type":"m.room.member","state_key":"@fixture-user:example.invalid","sender":"@fixture-user:example.invalid","event_id":"$member:example.invalid","origin_server_ts":3,"content":{"membership":"join"}}]}},"extensions":{}}"#
                    } else {
                        r#"{"pos":"sliding-pos"}"#
                    }
                } else if text.contains("/_matrix/client/") && text.contains("/sync") {
                    r#"{"next_batch":"batch","device_lists":{"changed":[],"left":[]},"rooms":{"invite":{},"join":{},"leave":{},"knock":{}},"to_device":{"events":[]},"presence":{"events":[]},"account_data":{"events":[]},"device_one_time_keys_count":{}}"#
                } else {
                    r#"{"errcode":"M_NOT_FOUND","error":"not found"}"#
                };
                let body = if text
                    .contains("/_matrix/client/unstable/org.matrix.simplified_msc3575/sync")
                {
                    let mut response: serde_json::Value =
                        serde_json::from_str(body).expect("sliding-sync fixture response");
                    if let Some(txn_id) = text
                        .split_once("\r\n\r\n")
                        .and_then(|(_, body)| serde_json::from_str::<serde_json::Value>(body).ok())
                        .and_then(|request| request.get("txn_id").cloned())
                    {
                        response["txn_id"] = txn_id;
                    }
                    response.to_string()
                } else {
                    body.to_owned()
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(response.as_bytes()).expect("write");
            });
        }
    });

    let data_dir = tempfile::tempdir().expect("data tempdir");
    let credential_dir = tempfile::tempdir().expect("credential tempdir");
    let runtime = CoreRuntime::start_with_data_dir_and_file_credentials(
        data_dir.path().to_path_buf(),
        credential_dir.path().to_path_buf(),
    );
    let (probe_tx, mut probe_rx) = mpsc::unbounded_channel();
    assert!(
        runtime
            .account_actor_test_handle
            .send(AccountMessage::AttachLifecycleProbe { probe_tx })
            .await
    );
    let (trust_tx, trust_rx) = mpsc::unbounded_channel();
    let updates = futures_util::stream::unfold(trust_rx, |mut rx| async move {
        rx.recv().await.map(|trust| (trust, rx))
    });
    assert!(
        runtime
            .account_actor_test_handle
            .send(AccountMessage::ConfigureTrustObservation {
                observation: koushi_sdk::CurrentDeviceTrustObservation {
                    current: koushi_state::CurrentDeviceTrustState::Verified,
                    updates: Box::pin(updates),
                },
            })
            .await
    );
    let connection = runtime.attach();
    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Account(AccountCommand::LoginPassword {
            request_id,
            request: koushi_state::LoginRequest {
                homeserver,
                username: "fixture-user".to_owned(),
                password: koushi_state::AuthSecret::new("synthetic-password"),
                device_display_name: Some("Runtime Trust Test".to_owned()),
            },
            platform: koushi_state::DisplayPlatform::Linux,
        }))
        .await
        .expect("login command");

    wait_for_runtime_session(&runtime, "initial promotion", |session| {
        matches!(session, SessionState::Ready(_))
    })
    .await;
    wait_for_runtime_sync_running(&runtime, "initial promotion").await;
    assert_eq!(
        inspect_runtime_children(&runtime).await,
        (true, true, true, true)
    );
    assert_eq!(probe_rx.recv().await, Some("ready_projection_ack"));
    assert_eq!(
        probe_rx.try_recv(),
        Err(mpsc::error::TryRecvError::Empty),
        "trust observer must remain active in Ready without another lifecycle token"
    );

    trust_tx
        .send(koushi_state::CurrentDeviceTrustState::Unverified)
        .expect("verification-gate update");
    wait_for_runtime_session(&runtime, "trust revocation gate", |session| {
        matches!(
            session,
            SessionState::Provisional {
                phase: koushi_state::ProvisionalPhase::DiscoveringMethods,
                ..
            }
        )
    })
    .await;
    assert_eq!(
        inspect_runtime_children(&runtime).await,
        (true, false, false, true)
    );
    let mut tokens = Vec::new();
    while tokens.len() < 11 {
        tokens.push(probe_rx.recv().await.expect("stop token"));
    }
    assert_eq!(tokens[0], "gate_projection_ack");
    assert!(!tokens.contains(&"provisional_encryption_sync_terminated"));
    assert!(tokens.contains(&"stop_sync_actor"));
    assert!(tokens.contains(&"stop_timeline_manager"));
    assert!(tokens.contains(&"clear_room_session"));

    trust_tx
        .send(koushi_state::CurrentDeviceTrustState::Unverified)
        .expect("duplicate verification-gate update");
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(matches!(
        runtime.snapshot_rx.borrow().state.session,
        SessionState::Provisional {
            phase: koushi_state::ProvisionalPhase::DiscoveringMethods,
            ..
        }
    ));
    assert_eq!(
        probe_rx.try_recv(),
        Err(mpsc::error::TryRecvError::Empty),
        "duplicate trust must not restart the gate transition"
    );

    trust_tx
        .send(koushi_state::CurrentDeviceTrustState::Verified)
        .expect("repromotion update");
    wait_for_runtime_session(&runtime, "verified repromotion", |session| {
        matches!(session, SessionState::Ready(_))
    })
    .await;
    wait_for_runtime_sync_running(&runtime, "verified repromotion").await;
    assert_eq!(
        inspect_runtime_children(&runtime).await,
        (true, true, true, true)
    );
    assert_eq!(
        probe_rx.recv().await,
        Some("provisional_encryption_sync_terminated")
    );
    assert_eq!(probe_rx.recv().await, Some("ready_projection_ack"));

    let before = runtime.snapshot_rx.borrow().state.session.clone();
    assert!(
        runtime
            .account_actor_test_handle
            .send(AccountMessage::CurrentDeviceTrustChanged {
                generation: 0,
                trust: koushi_state::CurrentDeviceTrustState::Unverified,
            })
            .await
    );
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(
        runtime.snapshot_rx.borrow().state.session,
        before,
        "stale/wrong-account trust changed state"
    );
    runtime.shutdown_handle().abort();
}

async fn wait_for_app_actor_shutdown(runtime: &CoreRuntime) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while !runtime.shutdown_handle().is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("AppActor shutdown handle should complete");
}

#[tokio::test]
async fn signed_out_shutdown_completes_app_actor_shutdown_handle() {
    let data_dir = tempfile::tempdir().expect("runtime data dir");
    let runtime = CoreRuntime::start_with_data_dir(data_dir.path().to_owned());
    let connection = runtime.attach();
    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::App(AppCommand::Shutdown { request_id }))
        .await
        .expect("signed-out shutdown command");

    wait_for_app_actor_shutdown(&runtime).await;
    runtime.shutdown().await;
}

#[tokio::test(flavor = "current_thread")]
async fn first_shutdown_publishes_preceding_state_and_ignores_duplicate_and_later_commands() {
    let data_dir = tempfile::tempdir().expect("runtime data dir");
    let runtime = CoreRuntime::start_with_data_dir(data_dir.path().to_owned());
    let mut connection = runtime.attach();
    let first_request_id = connection.next_request_id();
    let shutdown_request_id = connection.next_request_id();
    let duplicate_shutdown_request_id = connection.next_request_id();
    let later_request_id = connection.next_request_id();

    runtime
        .command_tx
        .send(CoreCommandEnvelope::Public {
            command: CoreCommand::App(AppCommand::UpdateSettings {
                request_id: first_request_id,
                patch: SettingsPatch {
                    thread_list_order: Some(koushi_state::ThreadListOrder::RootChronology),
                    ..SettingsPatch::default()
                },
            }),
            composer_permit: None,
            admission: None,
        })
        .await
        .expect("preceding command");
    runtime
        .command_tx
        .send(CoreCommandEnvelope::Public {
            command: CoreCommand::App(AppCommand::Shutdown {
                request_id: shutdown_request_id,
            }),
            composer_permit: None,
            admission: None,
        })
        .await
        .expect("first shutdown command");
    runtime
        .command_tx
        .send(CoreCommandEnvelope::Public {
            command: CoreCommand::App(AppCommand::Shutdown {
                request_id: duplicate_shutdown_request_id,
            }),
            composer_permit: None,
            admission: None,
        })
        .await
        .expect("duplicate shutdown command");
    runtime
        .command_tx
        .send(CoreCommandEnvelope::Public {
            command: CoreCommand::App(AppCommand::UpdateSettings {
                request_id: later_request_id,
                patch: SettingsPatch {
                    room_list_sort: Some(koushi_state::RoomListSort::RecentFirst),
                    ..SettingsPatch::default()
                },
            }),
            composer_permit: None,
            admission: None,
        })
        .await
        .expect("later command");

    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if matches!(
                connection.recv_event().await,
                Ok(CoreEvent::StateDelta(ref delta)) if delta.changed.settings.is_some()
            ) {
                break;
            }
        }
    })
    .await
    .expect("preceding settings delta must publish before shutdown completes");
    wait_for_app_actor_shutdown(&runtime).await;
    let snapshot = runtime.snapshot_rx.borrow();
    assert_eq!(
        snapshot.state.settings.values.thread_list_order,
        koushi_state::ThreadListOrder::RootChronology
    );
    assert_eq!(
        snapshot.state.settings.values.room_list_sort,
        koushi_state::RoomListSort::Activity,
        "commands queued after the first Shutdown must not be handled"
    );
    drop(snapshot);
    runtime.shutdown().await;
}

#[tokio::test]
async fn explicit_shutdown_is_a_barrier_before_same_data_dir_reopen() {
    let data_dir = tempfile::tempdir().expect("runtime data dir");
    let runtime = CoreRuntime::start_with_data_dir(data_dir.path().to_owned());
    let connection = runtime.attach();
    drop(connection);
    tokio::time::timeout(Duration::from_secs(3), runtime.shutdown())
        .await
        .expect("first runtime shutdown barrier");

    let reopened = CoreRuntime::start_with_data_dir(data_dir.path().to_owned());
    let connection = reopened.attach();
    drop(connection);
    tokio::time::timeout(Duration::from_secs(3), reopened.shutdown())
        .await
        .expect("reopened runtime shutdown barrier");
}

async fn inspect_runtime_children(runtime: &CoreRuntime) -> (bool, bool, bool, bool) {
    let (response, result) = oneshot::channel();
    assert!(
        runtime
            .account_actor_test_handle
            .send(AccountMessage::InspectSessionRuntime { response })
            .await
    );
    result.await.expect("runtime inspection")
}

async fn wait_for_runtime_session(
    runtime: &CoreRuntime,
    stage: &'static str,
    predicate: impl Fn(&SessionState) -> bool,
) {
    let mut snapshot_rx = runtime.snapshot_rx.clone();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if predicate(&snapshot_rx.borrow().state.session) {
                return;
            }
            snapshot_rx
                .changed()
                .await
                .unwrap_or_else(|_| panic!("snapshot channel closed during {stage}"));
        }
    })
    .await
    .unwrap_or_else(|_| panic!("session transition timed out during {stage}"));
}

async fn wait_for_runtime_sync_running(runtime: &CoreRuntime, stage: &'static str) {
    let mut snapshot_rx = runtime.snapshot_rx.clone();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if matches!(
                snapshot_rx.borrow().state.sync,
                koushi_state::SyncState::Running
            ) {
                return;
            }
            snapshot_rx
                .changed()
                .await
                .unwrap_or_else(|_| panic!("snapshot channel closed during {stage}"));
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "sync start timed out during {stage}: {:?}",
            runtime.snapshot_rx.borrow().state.sync
        )
    });
}
