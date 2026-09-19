use koushi_state::{
    AppAction, AppState, MAX_MENTION_CANDIDATE_TARGETS, MentionCandidate,
    MentionCandidateMembership, MentionCandidatesCompleteness, MentionCandidatesFailureKind,
    MentionSurface, RoomMentionPermission, SessionInfo, SessionState, reduce,
};

fn candidate(user_id: &str, display_label: Option<&str>) -> MentionCandidate {
    MentionCandidate {
        user_id: user_id.to_owned(),
        display_label: display_label.map(str::to_owned),
        original_display_label: display_label.map(str::to_owned),
        avatar: None,
        membership: MentionCandidateMembership::Joined,
    }
}

fn demand(
    request_id: u64,
    generation: u64,
    room_id: &str,
    surface: MentionSurface,
    query: &str,
) -> AppAction {
    AppAction::MentionCandidatesDemanded {
        request_id,
        generation,
        room_id: room_id.to_owned(),
        surface,
        query: query.to_owned(),
    }
}

fn projection(
    request_id: u64,
    generation: u64,
    room_id: &str,
    surface: MentionSurface,
    query: &str,
    completeness: MentionCandidatesCompleteness,
    candidates: Vec<MentionCandidate>,
) -> AppAction {
    AppAction::MentionCandidatesProjected {
        request_id,
        generation,
        room_id: room_id.to_owned(),
        surface,
        query: query.to_owned(),
        completeness,
        candidates,
        room_mention_allowed: RoomMentionPermission::Denied,
    }
}

#[test]
fn main_thread_and_edit_targets_are_independent_and_replace_only_their_exact_key() {
    let mut state = AppState::default();

    reduce(
        &mut state,
        demand(1, 1, "!room:test", MentionSurface::Main, "ali"),
    );
    reduce(
        &mut state,
        demand(2, 1, "!room:test", MentionSurface::Thread, "bob"),
    );
    reduce(
        &mut state,
        demand(3, 1, "!room:test", MentionSurface::Edit, "carol"),
    );
    reduce(
        &mut state,
        demand(4, 2, "!room:test", MentionSurface::Main, "alice"),
    );

    assert_eq!(state.mention_candidates.targets.len(), 3);
    let main = state
        .mention_candidates
        .target("!room:test", MentionSurface::Main)
        .unwrap();
    assert_eq!(main.request_id, 4);
    assert_eq!(main.generation, 2);
    assert_eq!(main.query, "alice");
    assert_eq!(main.completeness, MentionCandidatesCompleteness::Loading);
    let thread = state
        .mention_candidates
        .target("!room:test", MentionSurface::Thread)
        .unwrap();
    assert_eq!(thread.request_id, 2);
    assert_eq!(thread.query, "bob");
    let edit = state
        .mention_candidates
        .target("!room:test", MentionSurface::Edit)
        .unwrap();
    assert_eq!(edit.request_id, 3);
    assert_eq!(edit.query, "carol");
}

#[test]
fn partial_projection_keeps_only_the_explicit_joined_candidates_and_missing_label() {
    let mut state = AppState::default();
    reduce(
        &mut state,
        demand(4, 1, "!room:test", MentionSurface::Main, "member"),
    );
    reduce(
        &mut state,
        projection(
            4,
            1,
            "!room:test",
            MentionSurface::Main,
            "member",
            MentionCandidatesCompleteness::Partial,
            vec![
                candidate("@known:test", Some("Known Member")),
                candidate("@missing:test", None),
            ],
        ),
    );

    let target = state
        .mention_candidates
        .target("!room:test", MentionSurface::Main)
        .unwrap();
    assert_eq!(target.completeness, MentionCandidatesCompleteness::Partial);
    assert_eq!(target.candidates.len(), 2);
    assert_eq!(
        target.candidates[0].membership,
        MentionCandidateMembership::Joined
    );
    assert_eq!(target.candidates[1].display_label, None);
}

#[test]
fn stale_request_generation_query_and_room_completions_are_ignored() {
    let mut state = AppState::default();
    reduce(
        &mut state,
        demand(10, 1, "!room-a:test", MentionSurface::Main, "old"),
    );
    reduce(
        &mut state,
        demand(11, 2, "!room-a:test", MentionSurface::Main, "new"),
    );

    for stale in [
        projection(
            10,
            1,
            "!room-a:test",
            MentionSurface::Main,
            "old",
            MentionCandidatesCompleteness::Complete,
            vec![candidate("@old:test", Some("Old"))],
        ),
        projection(
            11,
            1,
            "!room-a:test",
            MentionSurface::Main,
            "new",
            MentionCandidatesCompleteness::Complete,
            vec![candidate("@old-generation:test", Some("Old generation"))],
        ),
        projection(
            11,
            2,
            "!room-a:test",
            MentionSurface::Main,
            "wrong-query",
            MentionCandidatesCompleteness::Complete,
            vec![candidate("@wrong-query:test", Some("Wrong query"))],
        ),
        projection(
            11,
            2,
            "!room-b:test",
            MentionSurface::Main,
            "new",
            MentionCandidatesCompleteness::Complete,
            vec![candidate("@wrong-room:test", Some("Wrong room"))],
        ),
    ] {
        reduce(&mut state, stale);
    }

    let target = state
        .mention_candidates
        .target("!room-a:test", MentionSurface::Main)
        .unwrap();
    assert_eq!(target.completeness, MentionCandidatesCompleteness::Loading);
    assert!(target.candidates.is_empty());
    assert!(
        state
            .mention_candidates
            .target("!room-b:test", MentionSurface::Main)
            .is_none()
    );
}

#[test]
fn matching_failure_settles_coarsely_and_stale_failure_is_ignored() {
    let mut state = AppState::default();
    reduce(
        &mut state,
        demand(20, 7, "!room:test", MentionSurface::Thread, "q"),
    );
    reduce(
        &mut state,
        AppAction::MentionCandidatesFailed {
            request_id: 19,
            generation: 6,
            room_id: "!room:test".to_owned(),
            surface: MentionSurface::Thread,
            query: "q".to_owned(),
            kind: MentionCandidatesFailureKind::Network,
        },
    );
    assert_eq!(
        state
            .mention_candidates
            .target("!room:test", MentionSurface::Thread)
            .unwrap()
            .completeness,
        MentionCandidatesCompleteness::Loading
    );

    reduce(
        &mut state,
        AppAction::MentionCandidatesFailed {
            request_id: 20,
            generation: 7,
            room_id: "!room:test".to_owned(),
            surface: MentionSurface::Thread,
            query: "q".to_owned(),
            kind: MentionCandidatesFailureKind::Sdk,
        },
    );
    let target = state
        .mention_candidates
        .target("!room:test", MentionSurface::Thread)
        .unwrap();
    assert_eq!(target.completeness, MentionCandidatesCompleteness::Failed);
    assert_eq!(target.failure_kind, Some(MentionCandidatesFailureKind::Sdk));
}

#[test]
fn target_collection_is_bounded_and_evicts_the_oldest_recent_target() {
    let mut state = AppState::default();
    for index in 0..=MAX_MENTION_CANDIDATE_TARGETS {
        reduce(
            &mut state,
            demand(
                index as u64,
                1,
                &format!("!room-{index}:test"),
                MentionSurface::Main,
                "q",
            ),
        );
    }

    assert_eq!(
        state.mention_candidates.targets.len(),
        MAX_MENTION_CANDIDATE_TARGETS
    );
    assert!(
        state
            .mention_candidates
            .target("!room-0:test", MentionSurface::Main)
            .is_none()
    );
    assert!(
        state
            .mention_candidates
            .target(
                &format!("!room-{MAX_MENTION_CANDIDATE_TARGETS}:test"),
                MentionSurface::Main,
            )
            .is_some()
    );
}

#[test]
fn logout_lock_and_account_switch_clear_mention_targets() {
    let session = SessionInfo {
        homeserver: "https://example.test".to_owned(),
        user_id: "@self:example.test".to_owned(),
        device_id: "DEVICE".to_owned(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    };

    for clear_action in [
        AppAction::LogoutRequested,
        AppAction::SessionLocked,
        AppAction::SwitchAccountRequested {
            info: SessionInfo {
                homeserver: "https://other.test".to_owned(),
                user_id: "@other:other.test".to_owned(),
                device_id: "OTHER".to_owned(),
                authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
            },
        },
    ] {
        let mut state = AppState {
            session: SessionState::Ready(session.clone()),
            ..AppState::default()
        };
        reduce(
            &mut state,
            demand(30, 1, "!room:test", MentionSurface::Main, "q"),
        );
        assert!(!state.mention_candidates.targets.is_empty());

        reduce(&mut state, clear_action);
        assert!(state.mention_candidates.targets.is_empty());
    }
}

#[test]
fn debug_output_redacts_room_query_user_and_labels() {
    let action = projection(
        40,
        2,
        "!private-room:test",
        MentionSurface::Main,
        "private-query",
        MentionCandidatesCompleteness::Complete,
        vec![candidate("@private-user:test", Some("Private Label"))],
    );
    let debug = format!("{action:?}");
    for private_value in [
        "!private-room:test",
        "private-query",
        "@private-user:test",
        "Private Label",
    ] {
        assert!(!debug.contains(private_value));
    }

    let mut state = AppState::default();
    reduce(
        &mut state,
        demand(
            40,
            2,
            "!private-room:test",
            MentionSurface::Main,
            "private-query",
        ),
    );
    reduce(&mut state, action);
    let debug = format!("{:?}", state.mention_candidates);
    assert!(debug.contains("target_count"));
    assert!(!debug.contains("private"));
}
