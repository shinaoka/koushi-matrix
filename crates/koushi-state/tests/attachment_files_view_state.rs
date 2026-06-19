use koushi_state::{
    AppAction, AppEffect, AppState, AttachmentFilter, AttachmentKind, AttachmentResult,
    AttachmentScope, AttachmentSort, FilesViewScope, FilesViewState, SessionInfo, SessionState,
    SpaceSummary, UiEvent, reduce,
};

fn session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://matrix.example.invalid".to_owned(),
        user_id: "@user-a:example.invalid".to_owned(),
        device_id: "DEVICE".to_owned(),
    }
}

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(session_info()),
        ..AppState::default()
    }
}

fn room_scope() -> FilesViewScope {
    FilesViewScope::Room {
        room_id: "!room:example.invalid".to_owned(),
    }
}

fn resolved_room_scope() -> AttachmentScope {
    AttachmentScope::Room {
        room_id: "!room:example.invalid".to_owned(),
    }
}

fn filter() -> AttachmentFilter {
    AttachmentFilter {
        kinds: vec![AttachmentKind::Image, AttachmentKind::File],
        filename_query: Some("report".to_owned()),
    }
}

fn sort() -> AttachmentSort {
    AttachmentSort::NewestFirst
}

fn attachment(event_id: &str) -> AttachmentResult {
    AttachmentResult {
        event_id: event_id.to_owned(),
        filename: "quarterly_report.pdf".to_owned(),
        kind: AttachmentKind::File,
        mimetype: Some("application/pdf".to_owned()),
        room_id: "!room:example.invalid".to_owned(),
        sender: "@user-a:example.invalid".to_owned(),
        size: Some(12_345),
        source_mxc: "mxc://example.invalid/synthetic-source".to_owned(),
        thumbnail_mxc: None,
        timestamp_ms: 1_700_000_000_000,
        thread_root: Some("$root:example.invalid".to_owned()),
        encrypted: false,
        encryption_version: None,
        width: None,
        height: None,
        is_edited: false,
    }
}

#[test]
fn default_files_view_is_closed() {
    let state = AppState::default();
    assert_eq!(state.files_view, FilesViewState::Closed);
}

#[test]
fn files_view_opened_transitions_to_loading_and_emits_search_effect() {
    let mut state = ready_state();

    let effects = reduce(
        &mut state,
        AppAction::FilesViewOpened {
            request_id: 7,
            scope: room_scope(),
            filter: filter(),
            sort: sort(),
        },
    );

    assert_eq!(
        state.files_view,
        FilesViewState::Loading {
            request_id: 7,
            scope: resolved_room_scope(),
            filter: filter(),
            sort: sort(),
        }
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::SearchAttachments {
                request_id: 7,
                scope: resolved_room_scope(),
                filter: filter(),
                sort: sort(),
            },
            AppEffect::EmitUiEvent(UiEvent::FilesViewChanged),
        ]
    );
}

#[test]
fn files_view_query_succeeded_transitions_to_open_with_results() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::FilesViewOpened {
            request_id: 8,
            scope: room_scope(),
            filter: filter(),
            sort: sort(),
        },
    );

    let items = vec![attachment("$event-a:example.invalid")];
    let effects = reduce(
        &mut state,
        AppAction::FilesViewQuerySucceeded {
            request_id: 8,
            items: items.clone(),
        },
    );

    assert_eq!(
        state.files_view,
        FilesViewState::Open {
            request_id: 8,
            scope: resolved_room_scope(),
            filter: filter(),
            sort: sort(),
            items,
            selected_event_id: None,
        }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::FilesViewChanged)]
    );
}

#[test]
fn files_view_query_failed_transitions_to_failed() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::FilesViewOpened {
            request_id: 9,
            scope: room_scope(),
            filter: filter(),
            sort: sort(),
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::FilesViewQueryFailed {
            request_id: 9,
            message: "attachment index unavailable".to_owned(),
        },
    );

    assert_eq!(
        state.files_view,
        FilesViewState::Failed {
            request_id: 9,
            scope: resolved_room_scope(),
            filter: filter(),
            sort: sort(),
            message: "attachment index unavailable".to_owned(),
        }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::FilesViewChanged)]
    );
}

#[test]
fn files_view_closed_resets_to_closed() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::FilesViewOpened {
            request_id: 10,
            scope: room_scope(),
            filter: filter(),
            sort: sort(),
        },
    );

    let effects = reduce(&mut state, AppAction::FilesViewClosed);

    assert_eq!(state.files_view, FilesViewState::Closed);
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::FilesViewChanged)]
    );
}

#[test]
fn files_view_close_while_closed_is_no_op() {
    let mut state = ready_state();

    let effects = reduce(&mut state, AppAction::FilesViewClosed);

    assert_eq!(state.files_view, FilesViewState::Closed);
    assert_eq!(effects, Vec::<AppEffect>::new());
}

#[test]
fn stale_query_success_is_ignored() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::FilesViewOpened {
            request_id: 11,
            scope: room_scope(),
            filter: filter(),
            sort: sort(),
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::FilesViewQuerySucceeded {
            request_id: 99,
            items: vec![attachment("$stale:example.invalid")],
        },
    );

    assert_eq!(
        state.files_view,
        FilesViewState::Loading {
            request_id: 11,
            scope: resolved_room_scope(),
            filter: filter(),
            sort: sort(),
        }
    );
    assert_eq!(effects, Vec::<AppEffect>::new());
}

#[test]
fn stale_query_failure_is_ignored() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::FilesViewOpened {
            request_id: 12,
            scope: room_scope(),
            filter: filter(),
            sort: sort(),
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::FilesViewQueryFailed {
            request_id: 98,
            message: "stale failure".to_owned(),
        },
    );

    assert_eq!(
        state.files_view,
        FilesViewState::Loading {
            request_id: 12,
            scope: resolved_room_scope(),
            filter: filter(),
            sort: sort(),
        }
    );
    assert_eq!(effects, Vec::<AppEffect>::new());
}

#[test]
fn space_scope_resolves_to_attachment_scope_with_child_room_ids() {
    let mut state = ready_state();
    state.spaces = vec![SpaceSummary {
        space_id: "!space:example.invalid".to_owned(),
        display_name: "Synthetic Space".to_owned(),
        avatar: None,
        child_room_ids: vec![
            "!room-a:example.invalid".to_owned(),
            "!room-b:example.invalid".to_owned(),
        ],
    }];

    let scope = FilesViewScope::Space {
        space_id: "!space:example.invalid".to_owned(),
    };
    let expected_scope = AttachmentScope::Space {
        space_id: "!space:example.invalid".to_owned(),
        child_room_ids: vec![
            "!room-a:example.invalid".to_owned(),
            "!room-b:example.invalid".to_owned(),
        ],
    };

    let effects = reduce(
        &mut state,
        AppAction::FilesViewOpened {
            request_id: 13,
            scope,
            filter: filter(),
            sort: sort(),
        },
    );

    assert_eq!(
        state.files_view,
        FilesViewState::Loading {
            request_id: 13,
            scope: expected_scope.clone(),
            filter: filter(),
            sort: sort(),
        }
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::SearchAttachments {
                request_id: 13,
                scope: expected_scope,
                filter: filter(),
                sort: sort(),
            },
            AppEffect::EmitUiEvent(UiEvent::FilesViewChanged),
        ]
    );
}

#[test]
fn account_scope_resolves_to_attachment_scope_account() {
    let mut state = ready_state();

    let effects = reduce(
        &mut state,
        AppAction::FilesViewOpened {
            request_id: 14,
            scope: FilesViewScope::Account,
            filter: AttachmentFilter::default(),
            sort: AttachmentSort::Filename,
        },
    );

    assert_eq!(
        state.files_view,
        FilesViewState::Loading {
            request_id: 14,
            scope: AttachmentScope::Account,
            filter: AttachmentFilter::default(),
            sort: AttachmentSort::Filename,
        }
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::SearchAttachments {
                request_id: 14,
                scope: AttachmentScope::Account,
                filter: AttachmentFilter::default(),
                sort: AttachmentSort::Filename,
            },
            AppEffect::EmitUiEvent(UiEvent::FilesViewChanged),
        ]
    );
}

#[test]
fn selection_changes_update_selected_attachment_id() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::FilesViewOpened {
            request_id: 15,
            scope: room_scope(),
            filter: filter(),
            sort: sort(),
        },
    );
    reduce(
        &mut state,
        AppAction::FilesViewQuerySucceeded {
            request_id: 15,
            items: vec![attachment("$event-a:example.invalid")],
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::FilesViewSelectionChanged {
            event_id: Some("$event-a:example.invalid".to_owned()),
        },
    );

    assert_eq!(
        state.files_view,
        FilesViewState::Open {
            request_id: 15,
            scope: resolved_room_scope(),
            filter: filter(),
            sort: sort(),
            items: vec![attachment("$event-a:example.invalid")],
            selected_event_id: Some("$event-a:example.invalid".to_owned()),
        }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::FilesViewChanged)]
    );

    let effects = reduce(
        &mut state,
        AppAction::FilesViewSelectionChanged { event_id: None },
    );

    assert_eq!(
        state.files_view,
        FilesViewState::Open {
            request_id: 15,
            scope: resolved_room_scope(),
            filter: filter(),
            sort: sort(),
            items: vec![attachment("$event-a:example.invalid")],
            selected_event_id: None,
        }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::FilesViewChanged)]
    );
}

#[test]
fn selection_change_without_open_files_view_is_no_op() {
    let mut state = ready_state();

    let effects = reduce(
        &mut state,
        AppAction::FilesViewSelectionChanged {
            event_id: Some("$event-a:example.invalid".to_owned()),
        },
    );

    assert_eq!(state.files_view, FilesViewState::Closed);
    assert_eq!(effects, Vec::<AppEffect>::new());
}

#[test]
fn files_view_actions_require_ready_session() {
    let mut state = AppState::default();

    assert_eq!(
        reduce(
            &mut state,
            AppAction::FilesViewOpened {
                request_id: 1,
                scope: room_scope(),
                filter: filter(),
                sort: sort(),
            },
        ),
        Vec::<AppEffect>::new()
    );
    assert_eq!(
        reduce(
            &mut state,
            AppAction::FilesViewQuerySucceeded {
                request_id: 1,
                items: vec![attachment("$event:example.invalid")],
            },
        ),
        Vec::<AppEffect>::new()
    );
    assert_eq!(
        reduce(
            &mut state,
            AppAction::FilesViewQueryFailed {
                request_id: 1,
                message: "no session".to_owned(),
            },
        ),
        Vec::<AppEffect>::new()
    );
    assert_eq!(state.files_view, FilesViewState::Closed);
}

#[test]
fn logout_clears_files_view_and_emits_ui_event() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::FilesViewOpened {
            request_id: 20,
            scope: room_scope(),
            filter: filter(),
            sort: sort(),
        },
    );
    reduce(
        &mut state,
        AppAction::FilesViewQuerySucceeded {
            request_id: 20,
            items: vec![attachment("$event-a:example.invalid")],
        },
    );

    let effects = reduce(&mut state, AppAction::LogoutRequested);

    assert_eq!(state.files_view, FilesViewState::Closed);
    assert!(
        effects.contains(&AppEffect::EmitUiEvent(UiEvent::FilesViewChanged)),
        "expected FilesViewChanged after logout, got {:?}",
        effects
    );
}
