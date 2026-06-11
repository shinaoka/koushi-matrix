use std::collections::VecDeque;

use matrix_desktop_key::SessionKeyId;
use matrix_desktop_search::SensitiveString;
use matrix_desktop_search::{SearchCandidate, SearchDocumentStore, SearchEdit, SearchableEvent};
use matrix_desktop_state::{
    AppAction, AppEffect, AppState, LoginFlow, RoomSummary, SearchResult, SearchScope, SessionInfo,
    SidebarModel, SpaceSummary, ThreadPaneState, compose_sidebar, reduce,
};
use serde::{Deserialize, Serialize};

pub const DEFAULT_HOMESERVER: &str = "https://matrix.org";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FakeDesktopBackendConfig {
    pub homeserver: String,
    pub user_id: String,
    pub device_id: String,
    pub restore_session: bool,
}

impl Default for FakeDesktopBackendConfig {
    fn default() -> Self {
        Self {
            homeserver: DEFAULT_HOMESERVER.to_owned(),
            user_id: "@demo-user:example.invalid".to_owned(),
            device_id: "FAKEDEVICE".to_owned(),
            restore_session: true,
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
    next_search_request_id: u64,
}

impl Default for FakeDesktopBackend {
    fn default() -> Self {
        Self::new(FakeDesktopBackendConfig::default())
    }
}

impl FakeDesktopBackend {
    pub fn new(config: FakeDesktopBackendConfig) -> Self {
        let timeline_messages = fixture_timeline_messages();
        let thread_replies = fixture_thread_replies();
        let (search_store, search_candidates) = fixture_search_store(&timeline_messages);

        Self {
            config,
            state: AppState::default(),
            search_store,
            search_candidates,
            timeline_messages,
            thread_replies,
            next_search_request_id: 1,
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
        self.dispatch(AppAction::OpenThread {
            room_id: DEFAULT_ROOM_ID.to_owned(),
            root_event_id: "$alpha-update".to_owned(),
        });
    }

    pub fn state(&self) -> &AppState {
        &self.state
    }

    pub fn snapshot(&self) -> DesktopSnapshot {
        let sidebar = compose_sidebar(
            self.state.navigation.active_space_id.as_deref(),
            &self.state.spaces,
            &self.state.rooms,
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
            matrix_desktop_state::SearchState::Results { results, .. } => results.clone(),
            _ => Vec::new(),
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
                    vec![AppAction::RestoreSessionSucceeded(self.session_info())]
                } else {
                    vec![AppAction::RestoreSessionNotFound]
                }
            }
            AppEffect::DiscoverLogin { homeserver } => {
                vec![AppAction::LoginDiscoverySucceeded {
                    homeserver: homeserver.clone(),
                    flows: fixture_login_flows(),
                }]
            }
            AppEffect::StartSync => self.start_fake_sync(),
            AppEffect::SubscribeTimeline { room_id } => vec![AppAction::TimelineSubscribed {
                room_id: room_id.clone(),
            }],
            AppEffect::OpenThreadTimeline {
                room_id,
                root_event_id,
            } => vec![AppAction::ThreadSubscribed {
                room_id: room_id.clone(),
                root_event_id: root_event_id.clone(),
            }],
            AppEffect::SearchMessages {
                request_id,
                query,
                scope,
            } => vec![AppAction::SearchSucceeded {
                request_id: *request_id,
                results: self.search(query, scope),
            }],
            AppEffect::SendText {
                room_id,
                transaction_id,
                body,
            } => {
                self.append_sent_message(room_id, transaction_id, body);
                vec![AppAction::SendTextFinished {
                    room_id: room_id.clone(),
                    transaction_id: transaction_id.clone(),
                }]
            }
            AppEffect::PersistSession(_)
            | AppEffect::ClearSession
            | AppEffect::StopSync
            | AppEffect::EmitUiEvent(_) => Vec::new(),
            AppEffect::Login(_) => vec![AppAction::LoginFailed {
                message: "real Matrix login is not wired in this pre-login foundation".to_owned(),
            }],
        }
    }

    fn session_info(&self) -> SessionInfo {
        SessionInfo {
            homeserver: self.config.homeserver.clone(),
            user_id: self.config.user_id.clone(),
            device_id: self.config.device_id.clone(),
        }
    }

    fn start_fake_sync(&self) -> Vec<AppAction> {
        let mut actions = vec![
            AppAction::SyncStarted,
            AppAction::RoomListUpdated {
                spaces: fixture_spaces(),
                rooms: fixture_rooms(),
            },
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

    fn search(&self, query: &str, scope: &SearchScope) -> Vec<SearchResult> {
        let mut results = self
            .search_candidates
            .iter()
            .filter(|candidate| self.room_is_in_scope(&candidate.room_id, scope))
            .filter_map(|candidate| self.search_store.verify_candidate(candidate.clone(), query))
            .collect::<Vec<_>>();

        results.sort_by(|left, right| {
            right
                .score_millis
                .cmp(&left.score_millis)
                .then_with(|| left.timestamp_ms.cmp(&right.timestamp_ms))
                .then_with(|| left.event_id.cmp(&right.event_id))
        });
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
        });
        self.search_candidates.push(SearchCandidate {
            room_id: room_id.to_owned(),
            event_id,
            score_millis: 500,
        });
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
            child_room_ids: vec![
                DEFAULT_ROOM_ID.to_owned(),
                "!room-planning:example.invalid".to_owned(),
            ],
        },
        SpaceSummary {
            space_id: "!space-beta:example.invalid".to_owned(),
            display_name: "Synthetic Lab".to_owned(),
            child_room_ids: vec!["!room-search:example.invalid".to_owned()],
        },
    ]
}

fn fixture_rooms() -> Vec<RoomSummary> {
    vec![
        RoomSummary {
            room_id: DEFAULT_ROOM_ID.to_owned(),
            display_name: "synthetic-room".to_owned(),
            is_dm: false,
            unread_count: 8,
            parent_space_ids: vec![DEFAULT_SPACE_ID.to_owned()],
        },
        RoomSummary {
            room_id: "!room-planning:example.invalid".to_owned(),
            display_name: "planning-room".to_owned(),
            is_dm: false,
            unread_count: 2,
            parent_space_ids: vec![DEFAULT_SPACE_ID.to_owned()],
        },
        RoomSummary {
            room_id: "!room-search:example.invalid".to_owned(),
            display_name: "matrix-sdk-search".to_owned(),
            is_dm: false,
            unread_count: 1,
            parent_space_ids: vec!["!space-beta:example.invalid".to_owned()],
        },
        RoomSummary {
            room_id: "!dm-member-1:example.invalid".to_owned(),
            display_name: "Member 1".to_owned(),
            is_dm: true,
            unread_count: 1,
            parent_space_ids: Vec::new(),
        },
        RoomSummary {
            room_id: "!dm-member-2:example.invalid".to_owned(),
            display_name: "Member 2".to_owned(),
            is_dm: true,
            unread_count: 0,
            parent_space_ids: Vec::new(),
        },
    ]
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

    matrix_desktop_auth::parse_login_discovery(&response)
        .expect("synthetic login discovery fixture should parse")
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
