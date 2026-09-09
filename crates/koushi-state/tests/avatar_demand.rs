use koushi_state::{
    AvatarDemandContext, AvatarDemandError, AvatarDemandState, VIEW_SCOPE_CAPACITY,
};

fn context() -> AvatarDemandContext {
    AvatarDemandContext {
        account_id: "@synthetic:example.invalid".into(),
        session_generation: 7,
    }
}

fn resources(values: &[&str]) -> Vec<Option<String>> {
    values.iter().map(|value| Some((*value).into())).collect()
}

#[test]
fn visible_resources_precede_prefetch_and_shared_demand_survives_one_scope_closing() {
    let context = context();
    let mut state = AvatarDemandState::new(context.clone());
    state.open(1).unwrap();
    state.open(2).unwrap();
    state
        .replace(
            &context,
            1,
            1,
            resources(&["shared"]),
            resources(&["prefetch"]),
        )
        .unwrap();
    state
        .replace(
            &context,
            2,
            1,
            resources(&["visible", "shared"]),
            resources(&["shared"]),
        )
        .unwrap();
    assert_eq!(
        state.resources_by_priority(),
        ["shared", "visible", "prefetch"]
    );
    assert!(state.close(1));
    assert_eq!(state.resources_by_priority(), ["visible", "shared"]);
    assert!(state.close(2));
    assert!(state.resources_by_priority().is_empty());
    assert_eq!(
        state.replace(&context, 1, 2, resources(&["late"]), vec![]),
        Err(AvatarDemandError::Closed)
    );
}

#[test]
fn stale_observations_and_account_session_changes_cannot_replace_live_demand() {
    let context = context();
    let mut state = AvatarDemandState::new(context.clone());
    state.open(1).unwrap();
    state
        .replace(&context, 1, 3, resources(&["current"]), vec![])
        .unwrap();
    for revision in [2, 3] {
        assert_eq!(
            state.replace(&context, 1, revision, resources(&["old"]), vec![]),
            Err(AvatarDemandError::StaleObservation)
        );
    }
    for changed in [
        AvatarDemandContext {
            session_generation: 8,
            ..context.clone()
        },
        AvatarDemandContext {
            account_id: "@other:example.invalid".into(),
            ..context.clone()
        },
    ] {
        assert_eq!(
            state.replace(&changed, 1, 4, resources(&["other"]), vec![]),
            Err(AvatarDemandError::SessionChanged)
        );
    }
    assert_eq!(state.resources_by_priority(), ["current"]);
}

#[test]
fn scope_and_window_limits_reject_atomically_without_counting_placeholders_as_resources() {
    let context = context();
    let mut state = AvatarDemandState::new(context.clone());
    for scope in 1..=VIEW_SCOPE_CAPACITY as u64 {
        state.open(scope).unwrap();
    }
    assert_eq!(
        state.open(VIEW_SCOPE_CAPACITY as u64 + 1),
        Err(AvatarDemandError::Capacity)
    );
    state
        .replace(&context, 1, 1, vec![None; 256], vec![None; 8])
        .unwrap();
    assert!(state.resources_by_priority().is_empty());
    for (visible, prefetch) in [(257, 0), (0, 9)] {
        assert_eq!(
            state.replace(&context, 1, 2, vec![None; visible], vec![None; prefetch]),
            Err(AvatarDemandError::Capacity)
        );
    }
    // Rejected input did not consume revision 2.
    state
        .replace(&context, 1, 2, resources(&["accepted"]), vec![])
        .unwrap();
    assert_eq!(state.resources_by_priority(), ["accepted"]);
}

#[test]
fn source_refresh_preserves_observation_order_and_rejects_overflow_and_closed_scopes() {
    let context = context();
    let mut state = AvatarDemandState::new(context.clone());
    state.open(1).unwrap();
    state
        .replace(&context, 1, 10, resources(&["old"]), vec![])
        .unwrap();
    let before = state.clone();
    for (visible, prefetch) in [(257, 0), (0, 9)] {
        assert_eq!(
            state.refresh(1, vec![None; visible], vec![None; prefetch]),
            Err(AvatarDemandError::Capacity)
        );
        assert!(state.same_resource_demand(&before));
    }
    assert_eq!(state.refresh(1, resources(&["new"]), vec![]), Ok(true));
    assert_eq!(before.resources_by_priority(), ["old"]);
    assert_eq!(state.resources_by_priority(), ["new"]);
    assert!(!state.same_resource_demand(&before));
    let refreshed = state.clone();
    assert_eq!(state.refresh(1, resources(&["new"]), vec![]), Ok(false));
    assert_eq!(
        state.replace(&context, 1, 10, resources(&["stale"]), vec![]),
        Err(AvatarDemandError::StaleObservation)
    );
    state
        .replace(&context, 1, 11, resources(&["new"]), vec![])
        .unwrap();
    assert!(
        state.same_resource_demand(&refreshed),
        "host counters are not resource interests"
    );
    state.open(2).unwrap();
    state
        .replace(&context, 2, 1, resources(&["new"]), vec![])
        .unwrap();
    assert!(
        !state.same_resource_demand(&refreshed),
        "a new consumer must still receive cached state"
    );
    state.close(1);
    assert_eq!(
        state.refresh(1, resources(&["late"]), vec![]),
        Err(AvatarDemandError::Closed)
    );
    assert_eq!(state.resources_by_priority(), ["new"]);
}

#[test]
fn demand_round_trips_but_debug_does_not_disclose_identities_or_resources() {
    let context = context();
    let mut state = AvatarDemandState::new(context.clone());
    state.open(1).unwrap();
    state
        .replace(
            &context,
            1,
            1,
            resources(&["mxc://example.invalid/synthetic-secret"]),
            vec![],
        )
        .unwrap();
    let mut next = state.clone();
    assert_eq!(
        state.resources_by_priority()[0].as_ptr(),
        next.resources_by_priority()[0].as_ptr(),
        "snapshot copies must share retained scope payload"
    );
    next.close(1);
    assert_eq!(state.resources_by_priority().len(), 1);
    let wire = serde_json::to_string(&state).unwrap();
    let restored: AvatarDemandState = serde_json::from_str(&wire).unwrap();
    assert_eq!(
        restored.resources_by_priority(),
        state.resources_by_priority()
    );
    let debug = format!("{context:?} {state:?}");
    for private in ["@synthetic", "example.invalid", "synthetic-secret"] {
        assert!(!debug.contains(private));
    }
}
