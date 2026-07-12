use std::collections::VecDeque;

use koushi_key::SessionKeyId;
use koushi_search::SensitiveString;
use koushi_search::{SearchDocumentStore, SearchEdit, SearchableEvent};
use koushi_state::{
    AppAction, AppEffect, AppState, AttachmentFilter, AttachmentResult, AttachmentScope,
    AttachmentSort, AuthFailureKind, DelegatedAuthLinks, LoginAttemptId, LoginFlow, LoginRequest,
    RecoveryMethod, RecoveryRequest, RoomSummary, RoomTags, SearchResult, SearchScope, SessionInfo,
    SidebarModel, SpaceSummary, ThreadPaneState, TrustOperationFailureKind,
    compose_sidebar_with_room_notification_settings, reduce,
};
use serde::{Deserialize, Serialize};

mod composition;

pub use composition::{
    DesktopRoomListRoom, DesktopRoomListSpace, DesktopRoomListUpdate, compose_room_list_update,
};
pub use koushi_search::SearchCandidate;

pub const DEFAULT_HOMESERVER: &str = "https://matrix.org";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FakeDesktopBackendConfig {
    pub homeserver: String,
    pub user_id: String,
    pub device_id: String,
    pub restore_session: bool,
    pub login_discovery: LoginDiscoveryMode,
    pub login: LoginMode,
    pub e2ee_recovery: E2eeRecoveryMode,
    pub sync: SyncMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum LoginDiscoveryMode {
    Fixture,
    Http,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum LoginMode {
    FixtureFailure,
    MatrixSdk,
    Deferred,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum E2eeRecoveryMode {
    NotRequired,
    SdkState,
    RequiredFixtureSuccess,
    RequiredDeferred,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SyncMode {
    Fixture,
    Deferred,
}

impl Default for FakeDesktopBackendConfig {
    fn default() -> Self {
        Self {
            homeserver: DEFAULT_HOMESERVER.to_owned(),
            user_id: "@demo-user:example.invalid".to_owned(),
            device_id: "FAKEDEVICE".to_owned(),
            restore_session: true,
            login_discovery: LoginDiscoveryMode::Fixture,
            login: LoginMode::FixtureFailure,
            e2ee_recovery: E2eeRecoveryMode::NotRequired,
            sync: SyncMode::Fixture,
        }
    }
}

pub struct FakeDesktopBackend {
    config: FakeDesktopBackendConfig,
    state: AppState,
    search_store: SearchDocumentStore,
    search_candidates: Vec<SearchCandidate>,
    timeline_messages: Vec<TimelineMessage>,
    thread_replies: Vec<ThreadMessage>,
    matrix_session: Option<koushi_sdk::MatrixClientSession>,
    active_login_attempt: Option<LoginAttemptId>,
    next_search_request_id: u64,
    backward_timeline_messages: Vec<TimelineMessage>,
}

impl Default for FakeDesktopBackend {
    fn default() -> Self {
        Self::new(FakeDesktopBackendConfig::default())
    }
}

impl FakeDesktopBackend {
    pub fn new(config: FakeDesktopBackendConfig) -> Self {
        let timeline_messages = fixture_timeline_messages();
        let backward_timeline_messages = fixture_backward_timeline_messages();
        let thread_replies = fixture_thread_replies();
        let (search_store, search_candidates) = fixture_search_store(&timeline_messages);

        Self {
            config,
            state: AppState::default(),
            search_store,
            search_candidates,
            timeline_messages,
            thread_replies,
            matrix_session: None,
            active_login_attempt: None,
            next_search_request_id: 1,
            backward_timeline_messages,
        }
    }

    pub fn booted() -> Self {
        let mut backend = Self::default();
        backend.boot();
        backend
    }

    pub fn booted_with_config(config: FakeDesktopBackendConfig) -> Self {
        let mut backend = Self::new(config);
        backend.boot();
        backend
    }

    pub fn boot(&mut self) {
        self.dispatch(AppAction::AppStarted);
        self.open_default_thread();
    }

    pub fn open_default_thread(&mut self) {
        self.dispatch(AppAction::OpenThread {
            room_id: DEFAULT_ROOM_ID.to_owned(),
            root_event_id: "$alpha-update".to_owned(),
        });
    }

    pub fn state(&self) -> &AppState {
        &self.state
    }

    pub fn snapshot(&self) -> DesktopSnapshot {
        let sidebar = compose_sidebar_with_room_notification_settings(
            self.state.navigation.active_space_id.as_deref(),
            &self.state.spaces,
            &self.state.rooms,
            &self.state.room_notification_settings,
        );
        let timeline = self
            .state
            .navigation
            .active_room_id
            .as_deref()
            .map(|room_id| {
                self.timeline_messages
                    .iter()
                    .filter(|message| message.room_id == room_id)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let thread = self.thread_snapshot();

        DesktopSnapshot {
            state: self.state.clone(),
            sidebar,
            timeline,
            thread,
        }
    }

    pub fn session_key_id(&self) -> SessionKeyId {
        SessionKeyId {
            homeserver: self.config.homeserver.clone(),
            user_id: self.config.user_id.clone(),
            device_id: self.config.device_id.clone(),
        }
    }

    pub fn matrix_session(&self) -> Option<koushi_sdk::MatrixClientSession> {
        self.matrix_session.clone()
    }

    pub fn observe_e2ee_recovery_state(
        &mut self,
        state: koushi_state::E2eeRecoveryState,
    ) -> Vec<AppEffect> {
        self.dispatch(AppAction::E2eeRecoveryStateChanged {
            state,
            methods: default_recovery_methods(),
        })
    }

    pub fn submit_search(
        &mut self,
        query: impl Into<String>,
        scope: SearchScope,
    ) -> Vec<SearchResult> {
        let query = query.into();
        let request_id = self.next_search_request_id;
        self.next_search_request_id += 1;

        self.dispatch(AppAction::SearchEdited {
            query: query.clone(),
            scope: scope.clone(),
        });
        self.dispatch(AppAction::SearchSubmitted {
            request_id,
            query,
            scope,
        });

        match &self.state.search {
            koushi_state::SearchState::Results { results, .. } => results.clone(),
            _ => Vec::new(),
        }
    }

    pub fn submit_search_candidates(
        &mut self,
        query: impl Into<String>,
        scope: SearchScope,
        candidates: Vec<SearchCandidate>,
    ) -> Vec<SearchResult> {
        let query = query.into();
        let request_id = self.next_search_request_id;
        self.next_search_request_id += 1;

        self.dispatch(AppAction::SearchEdited {
            query: query.clone(),
            scope: scope.clone(),
        });
        let _effects = reduce(
            &mut self.state,
            AppAction::SearchSubmitted {
                request_id,
                query: query.clone(),
                scope: scope.clone(),
            },
        );

        let results = self.search_candidates(&query, &scope, &candidates);
        self.dispatch(AppAction::SearchSucceeded {
            request_id,
            query,
            scope,
            results: results.clone(),
        });
        results
    }

    pub fn edit_message(&mut self, room_id: &str, event_id: &str, body: &str) {
        let Some(message) = self
            .timeline_messages
            .iter_mut()
            .find(|message| message.room_id == room_id && message.event_id == event_id)
        else {
            return;
        };

        message.body = body.to_owned();
        message.attachment_filename = None;
        self.search_store.upsert_edit(SearchEdit {
            edit_event_id: format!("{event_id}.edit"),
            target_event_id: event_id.to_owned(),
            sender: self.config.user_id.clone(),
            timestamp_ms: message.timestamp_ms + 1,
            body: Some(SensitiveString::new(body)),
            attachment_filename: None,
            attachment: None,
        });
    }

    pub fn redact_message(&mut self, room_id: &str, event_id: &str) {
        self.timeline_messages
            .retain(|message| !(message.room_id == room_id && message.event_id == event_id));
        self.search_store.redact(event_id);
        self.search_candidates
            .retain(|candidate| candidate.event_id != event_id);
    }

    pub fn upsert_timeline_messages(&mut self, messages: Vec<TimelineMessage>) {
        for message in messages {
            if let Some(existing) = self.timeline_messages.iter_mut().find(|existing| {
                existing.room_id == message.room_id && existing.event_id == message.event_id
            }) {
                *existing = message.clone();
            } else {
                self.timeline_messages.push(message.clone());
            }

            self.search_store.upsert_message(SearchableEvent {
                room_id: message.room_id.clone(),
                event_id: message.event_id.clone(),
                sender: message.sender.clone(),
                timestamp_ms: message.timestamp_ms,
                body: Some(SensitiveString::new(message.body.clone())),
                attachment_filename: message
                    .attachment_filename
                    .clone()
                    .map(SensitiveString::new),
                attachment: None,
            });
            if !self
                .search_candidates
                .iter()
                .any(|candidate| candidate.event_id == message.event_id)
            {
                self.search_candidates.push(SearchCandidate {
                    room_id: message.room_id,
                    event_id: message.event_id,
                    score_millis: 500,
                });
            }
        }

        self.timeline_messages
            .sort_by(|left, right| left.timestamp_ms.cmp(&right.timestamp_ms));
    }

    pub fn apply_timeline_updates(&mut self, updates: Vec<TimelineUpdate>) {
        for update in updates {
            match update {
                TimelineUpdate::Upsert(message) => {
                    self.upsert_timeline_messages(vec![message]);
                }
                TimelineUpdate::Remove { room_id, event_id } => {
                    self.redact_message(&room_id, &event_id);
                }
            }
        }
    }

    pub fn dispatch(&mut self, action: AppAction) -> Vec<AppEffect> {
        let mut emitted = Vec::new();
        let mut queue = VecDeque::from(reduce(&mut self.state, action));

        while let Some(effect) = queue.pop_front() {
            for follow_up in self.handle_effect(&effect) {
                queue.extend(reduce(&mut self.state, follow_up));
            }
            emitted.push(effect);
        }

        emitted
    }

    fn handle_effect(&mut self, effect: &AppEffect) -> Vec<AppAction> {
        match effect {
            AppEffect::RestoreSession => {
                if self.config.restore_session {
                    vec![self.restored_session_action(self.session_info())]
                } else {
                    vec![AppAction::RestoreSessionNotFound]
                }
            }
            AppEffect::DiscoverLogin { homeserver } => self.discover_login(homeserver),
            AppEffect::StartSync => self.start_sync(),
            AppEffect::SubscribeTimeline { room_id } => self.subscribe_timeline(room_id),
            AppEffect::PaginateTimelineBackwards { room_id } => match self.config.sync {
                SyncMode::Fixture => self.paginate_timeline_backwards(room_id),
                SyncMode::Deferred => Vec::new(),
            },
            AppEffect::OpenThreadTimeline {
                room_id,
                root_event_id,
            } => vec![AppAction::ThreadSubscribed {
                room_id: room_id.clone(),
                root_event_id: root_event_id.clone(),
            }],
            AppEffect::OpenFocusedTimeline { room_id, event_id } => {
                vec![AppAction::FocusedContextSubscribed {
                    room_id: room_id.clone(),
                    event_id: event_id.clone(),
                }]
            }
            AppEffect::SearchMessages {
                request_id,
                query,
                scope,
            } => vec![AppAction::SearchSucceeded {
                request_id: *request_id,
                query: query.clone(),
                scope: scope.clone(),
                results: self.search(query, scope),
            }],
            AppEffect::SearchAttachments {
                request_id,
                scope,
                filter,
                sort,
            } => vec![AppAction::FilesViewQuerySucceeded {
                request_id: *request_id,
                items: self.search_attachments(scope, filter, sort),
            }],
            AppEffect::SendText {
                room_id,
                transaction_id,
                body,
            } => match self.config.sync {
                SyncMode::Fixture => {
                    self.append_sent_message(room_id, transaction_id, body);
                    vec![AppAction::SendTextFinished {
                        room_id: room_id.clone(),
                        transaction_id: transaction_id.clone(),
                    }]
                }
                SyncMode::Deferred => Vec::new(),
            },
            AppEffect::PersistSession(_)
            | AppEffect::PersistSettings { .. }
            | AppEffect::PersistRoomPreferences { .. }
            | AppEffect::StopSync
            | AppEffect::EmitUiEvent(_)
            | AppEffect::SubscribeThreadsList { .. }
            | AppEffect::PaginateThreadsList { .. }
            | AppEffect::UnsubscribeThreadsList
            // The fake backend has no SearchActor; silently ignore crawler
            // notifications (there are no background crawls in tests).
            | AppEffect::NotifySearchCrawlerRoomsAvailable { .. }
            | AppEffect::InvalidateSearchCrawlerCache
            | AppEffect::RebuildSearchIndex => Vec::new(),
            AppEffect::RequestVerification { request_id, .. }
            | AppEffect::AcceptVerification { request_id }
            | AppEffect::ConfirmSasVerification { request_id } => {
                vec![AppAction::VerificationFailed {
                    request_id: *request_id,
                    kind: TrustOperationFailureKind::Sdk,
                }]
            }
            AppEffect::CancelVerification { .. } => Vec::new(),
            AppEffect::BootstrapCrossSigning { request_id } => {
                vec![AppAction::BootstrapCrossSigningFailed {
                    request_id: *request_id,
                    kind: TrustOperationFailureKind::Sdk,
                }]
            }
            AppEffect::EnableKeyBackup { request_id }
            | AppEffect::RestoreKeyBackup { request_id, .. } => {
                vec![AppAction::KeyBackupFailed {
                    request_id: *request_id,
                    kind: TrustOperationFailureKind::Sdk,
                }]
            }
            AppEffect::ResetIdentity { request_id } => {
                vec![AppAction::ResetIdentityFailed {
                    request_id: *request_id,
                    kind: TrustOperationFailureKind::Sdk,
                }]
            }
            AppEffect::CheckCurrentDeviceTrust
            | AppEffect::DiscoverVerificationMethods
            | AppEffect::BeginSessionVerification { .. }
            | AppEffect::RejectProvisionalSession => Vec::new(),
            AppEffect::Login {
                attempt_id,
                request,
            } => {
                self.active_login_attempt = Some(*attempt_id);
                self.login(*attempt_id, request)
            }
            AppEffect::RecoverE2ee(request) => self.recover_e2ee(request),
        }
    }

    fn session_info(&self) -> SessionInfo {
        SessionInfo {
            homeserver: self.config.homeserver.clone(),
            user_id: self.config.user_id.clone(),
            device_id: self.config.device_id.clone(),
        }
    }

    fn discover_login(&self, homeserver: &str) -> Vec<AppAction> {
        match self.config.login_discovery {
            LoginDiscoveryMode::Fixture => {
                vec![AppAction::LoginDiscoverySucceeded {
                    homeserver: homeserver.to_owned(),
                    flows: fixture_login_flows(),
                    delegated: DelegatedAuthLinks::default(),
                }]
            }
            LoginDiscoveryMode::Http => match koushi_sdk::discover_login_flows(homeserver) {
                Ok(discovery) => vec![AppAction::LoginDiscoverySucceeded {
                    homeserver: discovery.homeserver,
                    flows: discovery.flows,
                    delegated: discovery.delegated,
                }],
                Err(error) => vec![AppAction::LoginDiscoveryFailed {
                    homeserver: homeserver.to_owned(),
                    kind: login_discovery_failure_kind(&error),
                }],
            },
        }
    }

    fn login(&mut self, attempt_id: LoginAttemptId, request: &LoginRequest) -> Vec<AppAction> {
        match self.config.login {
            LoginMode::FixtureFailure => vec![AppAction::LoginFailed {
                attempt_id,
                message: "real Matrix login is not wired in this pre-login foundation".to_owned(),
            }],
            LoginMode::MatrixSdk => match koushi_sdk::login_with_password_blocking(request) {
                Ok(session) => {
                    let info = session.info.clone();
                    self.matrix_session = Some(session);
                    vec![self.authenticated_session_action(info)]
                }
                Err(error) => vec![AppAction::LoginFailed {
                    attempt_id,
                    message: error.to_string(),
                }],
            },
            LoginMode::Deferred => Vec::new(),
        }
    }

    pub fn complete_matrix_login(
        &mut self,
        session: koushi_sdk::MatrixClientSession,
    ) -> Vec<AppEffect> {
        let info = session.info.clone();
        self.matrix_session = Some(session);
        self.dispatch(self.authenticated_session_action(info))
    }

    pub fn complete_matrix_restore(
        &mut self,
        session: koushi_sdk::MatrixClientSession,
    ) -> Vec<AppEffect> {
        let info = session.info.clone();
        self.matrix_session = Some(session);
        self.dispatch(self.restored_session_action(info))
    }

    pub fn fail_login(&mut self, message: impl Into<String>) -> Vec<AppEffect> {
        let attempt_id = self
            .active_login_attempt
            .unwrap_or_else(|| LoginAttemptId::new(0));
        self.dispatch(AppAction::LoginFailed {
            attempt_id,
            message: message.into(),
        })
    }

    pub fn record_session_persistence_failure(
        &mut self,
        message: impl Into<String>,
    ) -> Vec<AppEffect> {
        self.dispatch(AppAction::SessionPersistenceFailed {
            message: message.into(),
        })
    }

    fn authenticated_session_action(&self, info: SessionInfo) -> AppAction {
        if self.e2ee_recovery_is_required() {
            AppAction::E2eeRecoveryRequired {
                info,
                methods: default_recovery_methods(),
            }
        } else {
            AppAction::LoginSucceeded {
                attempt_id: self
                    .active_login_attempt
                    .unwrap_or_else(|| LoginAttemptId::new(0)),
                info,
            }
        }
    }

    fn restored_session_action(&self, info: SessionInfo) -> AppAction {
        if self.e2ee_recovery_is_required() {
            AppAction::E2eeRecoveryRequired {
                info,
                methods: default_recovery_methods(),
            }
        } else {
            AppAction::RestoreSessionSucceeded(info)
        }
    }

    fn e2ee_recovery_is_required(&self) -> bool {
        matches!(
            self.config.e2ee_recovery,
            E2eeRecoveryMode::RequiredFixtureSuccess | E2eeRecoveryMode::RequiredDeferred
        ) || (self.config.e2ee_recovery == E2eeRecoveryMode::SdkState
            && self.matrix_session.as_ref().is_some_and(|session| {
                session.e2ee_recovery_state() == koushi_sdk::E2eeRecoveryState::Incomplete
            }))
    }

    fn recover_e2ee(&self, _request: &RecoveryRequest) -> Vec<AppAction> {
        match self.config.e2ee_recovery {
            E2eeRecoveryMode::NotRequired
            | E2eeRecoveryMode::SdkState
            | E2eeRecoveryMode::RequiredDeferred => Vec::new(),
            E2eeRecoveryMode::RequiredFixtureSuccess => vec![AppAction::E2eeRecoverySucceeded],
        }
    }

    fn start_sync(&self) -> Vec<AppAction> {
        match self.config.sync {
            SyncMode::Fixture => self.start_fake_sync(),
            SyncMode::Deferred => Vec::new(),
        }
    }

    fn start_fake_sync(&self) -> Vec<AppAction> {
        let mut actions = vec![
            AppAction::SyncStarted,
            compose_room_list_update(fixture_room_list_update()),
        ];

        if self.state.navigation.active_space_id.is_none() {
            actions.push(AppAction::SelectSpace {
                space_id: Some(DEFAULT_SPACE_ID.to_owned()),
            });
        }

        if self.state.navigation.active_room_id.is_none() {
            actions.push(AppAction::SelectRoom {
                room_id: DEFAULT_ROOM_ID.to_owned(),
            });
        }

        actions
    }

    fn subscribe_timeline(&self, room_id: &str) -> Vec<AppAction> {
        match self.config.sync {
            SyncMode::Fixture => vec![AppAction::TimelineSubscribed {
                room_id: room_id.to_owned(),
            }],
            SyncMode::Deferred => Vec::new(),
        }
    }

    fn search(&self, query: &str, scope: &SearchScope) -> Vec<SearchResult> {
        self.search_candidates(query, scope, &self.search_candidates)
    }

    fn search_attachments(
        &self,
        scope: &AttachmentScope,
        filter: &AttachmentFilter,
        sort: &AttachmentSort,
    ) -> Vec<AttachmentResult> {
        self.search_store.attachments(scope, filter, *sort)
    }

    fn search_candidates(
        &self,
        query: &str,
        scope: &SearchScope,
        candidates: &[SearchCandidate],
    ) -> Vec<SearchResult> {
        // #162: the SDK ngram index is only an accelerator. Union the supplied
        // candidates with a direct store scan via the shared matcher so any
        // indexed message is findable, then apply the product scope. This keeps
        // the fake backend faithful to the fixed core `SearchActor`.
        const CANDIDATE_LIMIT: usize = 50;
        // For a single-room scope, push the room filter into the scan so scoping
        // happens before the cap (matching the core path); otherwise other
        // rooms' matches could fill the cap and drop the target room's results.
        let room_filter = match scope {
            SearchScope::CurrentRoom { room_id } => Some(room_id.as_str()),
            _ => None,
        };
        let mut results = self.search_store.search_with_candidates(
            query,
            room_filter,
            candidates,
            CANDIDATE_LIMIT,
        );
        results.retain(|result| self.room_is_in_scope(&result.room_id, scope));
        results
    }

    fn room_is_in_scope(&self, room_id: &str, scope: &SearchScope) -> bool {
        match scope {
            SearchScope::CurrentRoom {
                room_id: current_room_id,
            } => room_id == current_room_id,
            SearchScope::CurrentSpace { space_id } => fixture_spaces()
                .iter()
                .find(|space| &space.space_id == space_id)
                .is_some_and(|space| {
                    space
                        .child_room_ids
                        .iter()
                        .any(|child_id| child_id == room_id)
                }),
            SearchScope::Dms => fixture_rooms()
                .iter()
                .any(|room| room.room_id == room_id && room.is_dm),
            SearchScope::AllRooms => true,
        }
    }

    fn thread_snapshot(&self) -> Option<ThreadSnapshot> {
        let (room_id, root_event_id) = match &self.state.thread {
            ThreadPaneState::Open {
                room_id,
                root_event_id,
                ..
            }
            | ThreadPaneState::Opening {
                room_id,
                root_event_id,
            } => (room_id, root_event_id),
            ThreadPaneState::Closed => return None,
        };

        Some(ThreadSnapshot {
            room_id: room_id.clone(),
            root_event_id: root_event_id.clone(),
            replies: self
                .thread_replies
                .iter()
                .filter(|reply| &reply.room_id == room_id && &reply.root_event_id == root_event_id)
                .cloned()
                .collect(),
        })
    }

    fn append_sent_message(&mut self, room_id: &str, transaction_id: &str, body: &str) {
        let event_id = format!("$local-{transaction_id}");
        let timestamp_ms = 1_820_000_000_000 + self.timeline_messages.len() as u64;
        self.timeline_messages.push(TimelineMessage {
            room_id: room_id.to_owned(),
            event_id: event_id.clone(),
            sender: self.config.user_id.clone(),
            timestamp_ms,
            body: body.to_owned(),
            attachment_filename: None,
            reply_count: 0,
        });
        self.search_store.upsert_message(SearchableEvent {
            room_id: room_id.to_owned(),
            event_id: event_id.clone(),
            sender: self.config.user_id.clone(),
            timestamp_ms,
            body: Some(SensitiveString::new(body)),
            attachment_filename: None,
            attachment: None,
        });
        self.search_candidates.push(SearchCandidate {
            room_id: room_id.to_owned(),
            event_id,
            score_millis: 500,
        });
    }

    fn paginate_timeline_backwards(&mut self, room_id: &str) -> Vec<AppAction> {
        let mut older_messages = Vec::new();
        self.backward_timeline_messages.retain(|message| {
            if message.room_id == room_id {
                older_messages.push(message.clone());
                false
            } else {
                true
            }
        });

        if !older_messages.is_empty() {
            let insert_at = self
                .timeline_messages
                .iter()
                .position(|message| message.room_id == room_id)
                .unwrap_or(self.timeline_messages.len());
            for (offset, message) in older_messages.into_iter().enumerate() {
                self.timeline_messages.insert(insert_at + offset, message);
            }
        }

        vec![AppAction::TimelineBackPaginationFinished {
            room_id: room_id.to_owned(),
        }]
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DesktopSnapshot {
    pub state: AppState,
    pub sidebar: SidebarModel,
    pub timeline: Vec<TimelineMessage>,
    pub thread: Option<ThreadSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelineMessage {
    pub room_id: String,
    pub event_id: String,
    pub sender: String,
    pub timestamp_ms: u64,
    pub body: String,
    pub attachment_filename: Option<String>,
    pub reply_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TimelineUpdate {
    Upsert(TimelineMessage),
    Remove { room_id: String, event_id: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ThreadSnapshot {
    pub room_id: String,
    pub root_event_id: String,
    pub replies: Vec<ThreadMessage>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ThreadMessage {
    pub room_id: String,
    pub root_event_id: String,
    pub event_id: String,
    pub sender: String,
    pub timestamp_ms: u64,
    pub body: String,
}

const DEFAULT_SPACE_ID: &str = "!space-alpha:example.invalid";
const DEFAULT_ROOM_ID: &str = "!room-alpha:example.invalid";

fn fixture_spaces() -> Vec<SpaceSummary> {
    vec![
        SpaceSummary {
            space_id: DEFAULT_SPACE_ID.to_owned(),
            display_name: "Synthetic Workspace".to_owned(),
            avatar: None,
            child_room_ids: vec![
                DEFAULT_ROOM_ID.to_owned(),
                "!room-planning:example.invalid".to_owned(),
            ],
        },
        SpaceSummary {
            space_id: "!space-beta:example.invalid".to_owned(),
            display_name: "Synthetic Lab".to_owned(),
            avatar: None,
            child_room_ids: vec!["!room-search:example.invalid".to_owned()],
        },
    ]
}

fn fixture_rooms() -> Vec<RoomSummary> {
    vec![
        RoomSummary {
            room_id: DEFAULT_ROOM_ID.to_owned(),
            display_name: "synthetic-room".to_owned(),
            display_label: "synthetic-room".to_owned(),
            original_display_label: "synthetic-room".to_owned(),
            avatar: None,
            is_dm: false,
            dm_user_ids: Vec::new(),
            tags: RoomTags::default(),
            unread_count: 8,
            notification_count: 8,
            highlight_count: 1,
            marked_unread: false,
            last_activity_ms: 0,
            latest_event: None,
            parent_space_ids: vec![DEFAULT_SPACE_ID.to_owned()],
            dm_space_ids: Vec::new(),
            is_encrypted: false,
            joined_members: 0,
        },
        RoomSummary {
            room_id: "!room-planning:example.invalid".to_owned(),
            display_name: "planning-room".to_owned(),
            display_label: "planning-room".to_owned(),
            original_display_label: "planning-room".to_owned(),
            avatar: None,
            is_dm: false,
            dm_user_ids: Vec::new(),
            tags: RoomTags::default(),
            unread_count: 2,
            notification_count: 2,
            highlight_count: 0,
            marked_unread: false,
            last_activity_ms: 0,
            latest_event: None,
            parent_space_ids: vec![DEFAULT_SPACE_ID.to_owned()],
            dm_space_ids: Vec::new(),
            is_encrypted: false,
            joined_members: 0,
        },
        RoomSummary {
            room_id: "!room-search:example.invalid".to_owned(),
            display_name: "matrix-sdk-search".to_owned(),
            display_label: "matrix-sdk-search".to_owned(),
            original_display_label: "matrix-sdk-search".to_owned(),
            avatar: None,
            is_dm: false,
            dm_user_ids: Vec::new(),
            tags: RoomTags::default(),
            unread_count: 1,
            notification_count: 1,
            highlight_count: 0,
            marked_unread: false,
            last_activity_ms: 0,
            latest_event: None,
            parent_space_ids: vec!["!space-beta:example.invalid".to_owned()],
            dm_space_ids: Vec::new(),
            is_encrypted: false,
            joined_members: 0,
        },
        RoomSummary {
            room_id: "!dm-member-1:example.invalid".to_owned(),
            display_name: "Member 1".to_owned(),
            display_label: "Member 1".to_owned(),
            original_display_label: "Member 1".to_owned(),
            avatar: None,
            is_dm: true,
            dm_user_ids: vec!["@member-1:example.invalid".to_owned()],
            tags: RoomTags::default(),
            unread_count: 1,
            notification_count: 1,
            highlight_count: 0,
            marked_unread: false,
            last_activity_ms: 0,
            latest_event: None,
            parent_space_ids: vec![
                DEFAULT_SPACE_ID.to_owned(),
                "!space-beta:example.invalid".to_owned(),
            ],
            dm_space_ids: Vec::new(),
            is_encrypted: false,
            joined_members: 0,
        },
        RoomSummary {
            room_id: "!dm-member-2:example.invalid".to_owned(),
            display_name: "Member 2".to_owned(),
            display_label: "Member 2".to_owned(),
            original_display_label: "Member 2".to_owned(),
            avatar: None,
            is_dm: true,
            dm_user_ids: vec!["@member-2:example.invalid".to_owned()],
            tags: RoomTags::default(),
            unread_count: 0,
            notification_count: 0,
            highlight_count: 0,
            marked_unread: false,
            last_activity_ms: 0,
            latest_event: None,
            parent_space_ids: Vec::new(),
            dm_space_ids: Vec::new(),
            is_encrypted: false,
            joined_members: 0,
        },
    ]
}

fn fixture_room_list_update() -> DesktopRoomListUpdate {
    DesktopRoomListUpdate {
        spaces: fixture_spaces()
            .into_iter()
            .map(|space| DesktopRoomListSpace {
                space_id: space.space_id,
                display_name: space.display_name,
            })
            .collect(),
        rooms: fixture_rooms()
            .into_iter()
            .map(|room| DesktopRoomListRoom {
                room_id: room.room_id,
                display_name: room.display_name,
                is_dm: room.is_dm,
                unread_count: room.unread_count,
                notification_count: room.notification_count,
                highlight_count: room.highlight_count,
                parent_space_ids: room.parent_space_ids,
                joined_members: room.joined_members,
            })
            .collect(),
    }
}

fn fixture_timeline_messages() -> Vec<TimelineMessage> {
    vec![
        TimelineMessage {
            room_id: DEFAULT_ROOM_ID.to_owned(),
            event_id: "$alpha-update".to_owned(),
            sender: "Demo Coordinator".to_owned(),
            timestamp_ms: 1_806_986_400_000,
            body: "Alpha keyword update from demo coordinator.".to_owned(),
            attachment_filename: None,
            reply_count: 2,
        },
        TimelineMessage {
            room_id: DEFAULT_ROOM_ID.to_owned(),
            event_id: "$agenda".to_owned(),
            sender: "Demo Coordinator".to_owned(),
            timestamp_ms: 1_806_990_000_000,
            body: "Synthetic planning note.\n\n- Fixture item one\n- Fixture item two".to_owned(),
            attachment_filename: None,
            reply_count: 0,
        },
        TimelineMessage {
            room_id: DEFAULT_ROOM_ID.to_owned(),
            event_id: "$budget-file".to_owned(),
            sender: "Slackbot".to_owned(),
            timestamp_ms: 1_806_993_600_000,
            body: "Budget spreadsheet attached.".to_owned(),
            attachment_filename: Some("fixture_budget.xlsx".to_owned()),
            reply_count: 0,
        },
        TimelineMessage {
            room_id: DEFAULT_ROOM_ID.to_owned(),
            event_id: "$false-positive".to_owned(),
            sender: "Member 3".to_owned(),
            timestamp_ms: 1_806_997_200_000,
            body: "Non-matching synthetic note.".to_owned(),
            attachment_filename: None,
            reply_count: 0,
        },
        TimelineMessage {
            room_id: "!room-planning:example.invalid".to_owned(),
            event_id: "$late-original".to_owned(),
            sender: "Member 1".to_owned(),
            timestamp_ms: 1_807_000_800_000,
            body: "Final synthetic checklist".to_owned(),
            attachment_filename: None,
            reply_count: 0,
        },
        TimelineMessage {
            room_id: "!room-search:example.invalid".to_owned(),
            event_id: "$search-dev-note".to_owned(),
            sender: "Member 4".to_owned(),
            timestamp_ms: 1_807_004_400_000,
            body: "matrix-sdk-search adapter review notes".to_owned(),
            attachment_filename: None,
            reply_count: 0,
        },
    ]
}

fn fixture_backward_timeline_messages() -> Vec<TimelineMessage> {
    vec![TimelineMessage {
        room_id: DEFAULT_ROOM_ID.to_owned(),
        event_id: "$alpha-history".to_owned(),
        sender: "Demo Coordinator".to_owned(),
        timestamp_ms: 1_806_982_800_000,
        body: "Older synthetic context from the selected room.".to_owned(),
        attachment_filename: None,
        reply_count: 0,
    }]
}

fn fixture_thread_replies() -> Vec<ThreadMessage> {
    vec![
        ThreadMessage {
            room_id: DEFAULT_ROOM_ID.to_owned(),
            root_event_id: "$alpha-update".to_owned(),
            event_id: "$thread-1".to_owned(),
            sender: "Member 2".to_owned(),
            timestamp_ms: 1_806_987_000_000,
            body: "Synthetic follow-up item one.".to_owned(),
        },
        ThreadMessage {
            room_id: DEFAULT_ROOM_ID.to_owned(),
            root_event_id: "$alpha-update".to_owned(),
            event_id: "$thread-2".to_owned(),
            sender: "Member 1".to_owned(),
            timestamp_ms: 1_806_987_600_000,
            body: "Synthetic follow-up item two.".to_owned(),
        },
    ]
}

fn fixture_search_store(
    messages: &[TimelineMessage],
) -> (SearchDocumentStore, Vec<SearchCandidate>) {
    let mut store = SearchDocumentStore::default();
    let mut candidates = Vec::new();

    store.upsert_edit(SearchEdit {
        edit_event_id: "$late-edit".to_owned(),
        target_event_id: "$late-original".to_owned(),
        sender: "Member 1".to_owned(),
        timestamp_ms: 1_807_001_200_000,
        body: Some(SensitiveString::new("Final synthetic checklist")),
        attachment_filename: None,
        attachment: None,
    });

    for message in messages {
        let body = if message.event_id == "$late-original" {
            "Original checklist placeholder".to_owned()
        } else {
            message.body.clone()
        };
        store.upsert_message(SearchableEvent {
            room_id: message.room_id.clone(),
            event_id: message.event_id.clone(),
            sender: message.sender.clone(),
            timestamp_ms: message.timestamp_ms,
            body: Some(SensitiveString::new(body)),
            attachment_filename: message
                .attachment_filename
                .as_ref()
                .map(|filename| SensitiveString::new(filename.clone())),
            attachment: None,
        });
        candidates.push(SearchCandidate {
            room_id: message.room_id.clone(),
            event_id: message.event_id.clone(),
            score_millis: candidate_score(&message.event_id),
        });
    }

    (store, candidates)
}

fn fixture_login_flows() -> Vec<LoginFlow> {
    let response = serde_json::json!({
        "flows": [
            { "type": "m.login.password" },
            {
                "type": "m.login.sso",
                "org.matrix.msc3824.delegated_oidc_compatibility": true
            }
        ]
    });

    koushi_sdk::parse_login_discovery(&response)
        .expect("synthetic login discovery fixture should parse")
}

fn login_discovery_failure_kind(error: &koushi_sdk::LoginDiscoveryError) -> AuthFailureKind {
    match error {
        koushi_sdk::LoginDiscoveryError::RequestFailed(_) => AuthFailureKind::Network,
        koushi_sdk::LoginDiscoveryError::HttpStatus { status: 403, .. } => {
            AuthFailureKind::Forbidden
        }
        koushi_sdk::LoginDiscoveryError::HttpStatus { .. }
        | koushi_sdk::LoginDiscoveryError::MissingFlows
        | koushi_sdk::LoginDiscoveryError::InvalidResponse(_) => AuthFailureKind::Sdk,
        koushi_sdk::LoginDiscoveryError::InvalidHomeserver(_)
        | koushi_sdk::LoginDiscoveryError::UnsupportedHomeserverScheme
        | koushi_sdk::LoginDiscoveryError::InsecureHomeserverScheme => AuthFailureKind::Unsupported,
    }
}

fn default_recovery_methods() -> Vec<RecoveryMethod> {
    vec![RecoveryMethod::RecoveryKey, RecoveryMethod::SecurityPhrase]
}

fn candidate_score(event_id: &str) -> u32 {
    match event_id {
        "$false-positive" => 1_000,
        "$alpha-update" => 950,
        "$budget-file" => 900,
        "$late-original" => 850,
        _ => 700,
    }
}
