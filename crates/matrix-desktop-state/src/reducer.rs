use crate::{
    action::AppAction,
    effect::{AppEffect, UiEvent},
    state::{
        AppError, AppState, NavigationState, SearchState, SessionState, SyncState, ThreadPaneState,
        TimelinePaneState,
    },
};

pub fn reduce(state: &mut AppState, action: AppAction) -> Vec<AppEffect> {
    match action {
        AppAction::AppStarted => {
            state.session = SessionState::Restoring;
            vec![AppEffect::RestoreSession]
        }
        AppAction::RestoreSessionSucceeded(info) | AppAction::LoginSucceeded(info) => {
            state.session = SessionState::Ready(info.clone());
            state.sync = SyncState::Starting;
            vec![
                AppEffect::PersistSession(info),
                AppEffect::StartSync,
                AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            ]
        }
        AppAction::RestoreSessionNotFound => {
            state.session = SessionState::SignedOut;
            vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
        }
        AppAction::RestoreSessionFailed { message } => {
            state.session = SessionState::SignedOut;
            state.errors.push(AppError {
                code: "restore_failed".to_owned(),
                message,
                recoverable: true,
            });
            vec![
                AppEffect::EmitUiEvent(UiEvent::SessionChanged),
                AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
            ]
        }
        AppAction::LoginSubmitted {
            homeserver,
            username,
        } => {
            state.session = SessionState::Authenticating {
                homeserver: homeserver.clone(),
            };
            vec![
                AppEffect::Login {
                    homeserver,
                    username,
                },
                AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            ]
        }
        AppAction::LoginFailed { message } => {
            state.session = SessionState::SignedOut;
            state.errors.push(AppError {
                code: "login_failed".to_owned(),
                message,
                recoverable: true,
            });
            vec![
                AppEffect::EmitUiEvent(UiEvent::SessionChanged),
                AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
            ]
        }
        AppAction::SessionLocked => {
            if let SessionState::Ready(info) = &state.session {
                state.session = SessionState::Locked(info.clone());
                state.sync = SyncState::Stopped;
                let mut effects = vec![
                    AppEffect::StopSync,
                    AppEffect::EmitUiEvent(UiEvent::SessionChanged),
                ];
                effects.extend(clear_session_views(state));
                return effects;
            }
            Vec::new()
        }
        AppAction::LogoutRequested => {
            state.session = SessionState::LoggingOut;
            state.sync = SyncState::Stopped;
            let mut effects = vec![
                AppEffect::StopSync,
                AppEffect::ClearSession,
                AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            ];
            effects.extend(clear_session_views(state));
            effects
        }
        AppAction::LogoutFinished => {
            *state = AppState::default();
            vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
        }
        AppAction::SyncStarted => {
            if !matches!(state.session, SessionState::Ready(_)) {
                return Vec::new();
            }

            match state.sync {
                SyncState::Starting | SyncState::Recovering { .. } => {
                    state.sync = SyncState::Running;
                    vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
                }
                SyncState::Running | SyncState::Stopped => Vec::new(),
            }
        }
        AppAction::SyncFailed { reason } => {
            if !matches!(state.session, SessionState::Ready(_))
                || matches!(state.sync, SyncState::Stopped)
            {
                return Vec::new();
            }

            state.sync = SyncState::Recovering { reason };
            vec![AppEffect::StartSync]
        }
        AppAction::SyncRecovered => {
            if !matches!(state.session, SessionState::Ready(_))
                || !matches!(state.sync, SyncState::Recovering { .. })
            {
                return Vec::new();
            }

            state.sync = SyncState::Running;
            vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
        }
        AppAction::SyncStopped => {
            if matches!(state.sync, SyncState::Stopped) {
                return Vec::new();
            }

            state.sync = SyncState::Stopped;
            vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
        }
        AppAction::RoomListUpdated { spaces, rooms } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            state.spaces = spaces;
            state.rooms = rooms;

            let mut effects = vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)];

            if state
                .navigation
                .active_space_id
                .as_deref()
                .is_some_and(|active_space_id| {
                    !state
                        .spaces
                        .iter()
                        .any(|space| space.space_id == active_space_id)
                })
            {
                state.navigation.active_space_id = None;
            }

            if let Some(active_room_id) = state.navigation.active_room_id.clone() {
                let room_still_exists = state
                    .rooms
                    .iter()
                    .any(|room| room.room_id == active_room_id);

                if !room_still_exists {
                    state.navigation.active_room_id = None;
                    let previous_room_id = state.timeline.room_id.clone().unwrap_or(active_room_id);
                    let had_thread = state.thread != ThreadPaneState::Closed;

                    state.timeline = Default::default();
                    state.thread = ThreadPaneState::Closed;

                    effects.push(AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
                        room_id: previous_room_id,
                    }));
                    if had_thread {
                        effects.push(AppEffect::EmitUiEvent(UiEvent::ThreadChanged));
                    }
                }
            }

            effects
        }
        AppAction::SelectSpace { space_id } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            state.navigation.active_space_id = space_id
                .filter(|space_id| state.spaces.iter().any(|space| space.space_id == *space_id));
            vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
        }
        AppAction::SelectRoom { room_id } => {
            if !is_session_ready(state) || !state.rooms.iter().any(|room| room.room_id == room_id) {
                return Vec::new();
            }

            let had_thread = state.thread != ThreadPaneState::Closed;
            state.navigation.active_room_id = Some(room_id.clone());
            state.timeline = TimelinePaneState {
                room_id: Some(room_id.clone()),
                is_subscribed: false,
                is_paginating_backwards: false,
                composer: Default::default(),
            };
            state.thread = ThreadPaneState::Closed;
            let mut effects = vec![
                AppEffect::SubscribeTimeline {
                    room_id: room_id.clone(),
                },
                AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id }),
            ];
            if had_thread {
                effects.push(AppEffect::EmitUiEvent(UiEvent::ThreadChanged));
            }
            effects
        }
        AppAction::TimelineSubscribed { room_id } => {
            if !is_session_ready(state)
                || state.timeline.room_id.as_deref() != Some(room_id.as_str())
            {
                return Vec::new();
            }

            state.timeline.is_subscribed = true;
            vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })]
        }
        AppAction::TimelineSubscriptionFailed { room_id, message } => {
            if !is_session_ready(state)
                || state.timeline.room_id.as_deref() != Some(room_id.as_str())
            {
                return Vec::new();
            }

            state.errors.push(AppError {
                code: "timeline_subscription_failed".to_owned(),
                message,
                recoverable: true,
            });
            vec![
                AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id }),
                AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
            ]
        }
        AppAction::ComposerDraftChanged { room_id, draft } => {
            if !is_session_ready(state)
                || state.timeline.room_id.as_deref() != Some(room_id.as_str())
            {
                return Vec::new();
            }

            state.timeline.composer.draft = draft;
            vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })]
        }
        AppAction::SendTextSubmitted {
            room_id,
            transaction_id,
            body,
        } => {
            if !is_session_ready(state)
                || state.timeline.room_id.as_deref() != Some(room_id.as_str())
                || state.timeline.composer.pending_transaction_id.is_some()
            {
                return Vec::new();
            }

            state.timeline.composer.pending_transaction_id = Some(transaction_id.clone());
            state.timeline.composer.draft.clear();
            vec![
                AppEffect::SendText {
                    room_id: room_id.clone(),
                    transaction_id,
                    body,
                },
                AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id }),
            ]
        }
        AppAction::SendTextFinished {
            room_id,
            transaction_id,
        } => {
            if !is_session_ready(state)
                || state.timeline.room_id.as_deref() != Some(room_id.as_str())
                || state.timeline.composer.pending_transaction_id.as_deref()
                    != Some(transaction_id.as_str())
            {
                return Vec::new();
            }

            state.timeline.composer.pending_transaction_id = None;
            vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })]
        }
        AppAction::OpenThread {
            room_id,
            root_event_id,
        } => {
            if !is_session_ready(state)
                || state.timeline.room_id.as_deref() != Some(room_id.as_str())
            {
                return Vec::new();
            }

            state.thread = ThreadPaneState::Opening {
                room_id: room_id.clone(),
                root_event_id: root_event_id.clone(),
            };
            vec![
                AppEffect::OpenThreadTimeline {
                    room_id,
                    root_event_id,
                },
                AppEffect::EmitUiEvent(UiEvent::ThreadChanged),
            ]
        }
        AppAction::ThreadSubscribed {
            room_id,
            root_event_id,
        } => {
            if !is_session_ready(state)
                || !matches!(
                    &state.thread,
                    ThreadPaneState::Opening {
                        room_id: opening_room_id,
                        root_event_id: opening_root_event_id,
                    } if opening_room_id == &room_id && opening_root_event_id == &root_event_id
                )
            {
                return Vec::new();
            }

            state.thread = ThreadPaneState::Open {
                room_id,
                root_event_id,
                is_subscribed: true,
                composer: Default::default(),
            };
            vec![AppEffect::EmitUiEvent(UiEvent::ThreadChanged)]
        }
        AppAction::CloseThread => {
            if !is_session_ready(state) || state.thread == ThreadPaneState::Closed {
                return Vec::new();
            }

            state.thread = ThreadPaneState::Closed;
            vec![AppEffect::EmitUiEvent(UiEvent::ThreadChanged)]
        }
        AppAction::SearchEdited { query, scope } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            state.search = SearchState::Editing { query, scope };
            vec![AppEffect::EmitUiEvent(UiEvent::SearchChanged)]
        }
        AppAction::SearchSubmitted {
            request_id,
            query,
            scope,
        } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            state.search = SearchState::Searching {
                request_id,
                query: query.clone(),
                scope: scope.clone(),
            };
            vec![
                AppEffect::SearchMessages {
                    request_id,
                    query,
                    scope,
                },
                AppEffect::EmitUiEvent(UiEvent::SearchChanged),
            ]
        }
        AppAction::SearchSucceeded {
            request_id,
            results,
        } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            let (current_request_id, query, scope) = match &state.search {
                SearchState::Searching {
                    request_id,
                    query,
                    scope,
                } => (*request_id, query.clone(), scope.clone()),
                _ => return Vec::new(),
            };

            if current_request_id != request_id {
                return Vec::new();
            }

            state.search = SearchState::Results {
                request_id,
                query,
                scope,
                results,
            };
            vec![AppEffect::EmitUiEvent(UiEvent::SearchChanged)]
        }
        AppAction::SearchFailed {
            request_id,
            message,
        } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            let (current_request_id, query, scope) = match &state.search {
                SearchState::Searching {
                    request_id,
                    query,
                    scope,
                } => (*request_id, query.clone(), scope.clone()),
                _ => return Vec::new(),
            };

            if current_request_id != request_id {
                return Vec::new();
            }

            state.search = SearchState::Failed {
                request_id,
                query,
                scope,
                message,
            };
            vec![AppEffect::EmitUiEvent(UiEvent::SearchChanged)]
        }
        AppAction::ClearError { code } => {
            state.errors.retain(|error| error.code != code);
            vec![AppEffect::EmitUiEvent(UiEvent::ErrorChanged)]
        }
    }
}

fn is_session_ready(state: &AppState) -> bool {
    matches!(state.session, SessionState::Ready(_))
}

fn clear_session_views(state: &mut AppState) -> Vec<AppEffect> {
    let previous_room_id = state.timeline.room_id.clone();
    let had_thread = state.thread != ThreadPaneState::Closed;
    let had_search = state.search != SearchState::Closed;

    state.navigation = NavigationState::default();
    state.spaces.clear();
    state.rooms.clear();
    state.timeline = Default::default();
    state.thread = ThreadPaneState::Closed;
    state.search = SearchState::Closed;

    let mut effects = vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)];
    if let Some(room_id) = previous_room_id {
        effects.push(AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id }));
    }
    if had_thread {
        effects.push(AppEffect::EmitUiEvent(UiEvent::ThreadChanged));
    }
    if had_search {
        effects.push(AppEffect::EmitUiEvent(UiEvent::SearchChanged));
    }
    effects
}
