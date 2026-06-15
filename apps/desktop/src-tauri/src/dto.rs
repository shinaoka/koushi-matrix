//! Data-transfer objects: Rust → TypeScript serialization contract.
//!
//! `FrontendDesktopSnapshot` is built from `AppStateSnapshot` (the core state
//! projection). Timeline items and thread messages are REMOVED from the
//! snapshot in Phase 7; they flow as `CoreEvent::Timeline` diffs over
//! `matrix-desktop://event`. The TS types.ts contract keeps `timeline` and
//! `thread` fields for backward compat; the adapter now always sends `[]` /
//! `null` and the React timeline store populates them from events.
//!
//! References: overview.md "Async rule 4" — timeline items never in AppState.

use std::collections::BTreeMap;

use matrix_desktop_state::{
    ActivityState, AppError, AppState, AuthDiscoveryState, BasicOperationState,
    CjkTextPolicyState, ComposerState, DirectoryState, DisplayPlatform, E2eeTrustState,
    FocusedContextState, InvitePreview, LiveSignalsState, LocalEncryptionState,
    LocaleDisplayProfile, NativeAttentionState, NavigationState, ProfileState, RecoveryMethod,
    RoomInteractionState, RoomManagementState, RoomSummary, SearchMatchField, SearchMatchKind,
    SearchResult, SearchScope, SearchState, SessionState, SettingsState, SidebarModel,
    SpaceSummary, SyncState, ThreadPaneState, TimelinePaneState, TypographyDisplayProfile,
    resolve_locale_display_profile, resolve_typography_display_profile,
};
use serde::{Deserialize, Serialize};

/// The snapshot returned by all Tauri commands.
///
/// `timeline` and `thread` are always empty / `None` in Phase 7; timeline
/// items flow as `TimelineEvent` diffs over `matrix-desktop://event`.
#[derive(Clone, Debug, Serialize)]
pub struct FrontendDesktopSnapshot {
    pub state: FrontendAppState,
    pub sidebar: SidebarModel,
    /// Always empty in Phase 7; timeline items flow as diffs.
    pub timeline: Vec<()>,
    /// Always None in Phase 7; thread flow as events.
    pub thread: Option<()>,
}

impl From<AppState> for FrontendDesktopSnapshot {
    fn from(state: AppState) -> Self {
        let sidebar = matrix_desktop_state::compose_sidebar(
            state.navigation.active_space_id.as_deref(),
            &state.spaces,
            &state.rooms,
        );
        Self {
            state: state.into(),
            sidebar,
            timeline: Vec::new(),
            thread: None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct FrontendAppState {
    pub session: FrontendSessionState,
    pub auth: AuthDiscoveryState,
    pub settings: SettingsState,
    pub locale_profile: LocaleDisplayProfile,
    pub typography_profile: TypographyDisplayProfile,
    pub profile: ProfileState,
    pub sync: FrontendSyncState,
    pub navigation: NavigationState,
    pub spaces: Vec<SpaceSummary>,
    pub rooms: Vec<RoomSummary>,
    pub invites: Vec<InvitePreview>,
    pub room_interactions: BTreeMap<String, RoomInteractionState>,
    pub directory: DirectoryState,
    pub room_management: RoomManagementState,
    pub activity: ActivityState,
    pub timeline: TimelinePaneState,
    pub thread: FrontendThreadPaneState,
    pub focused_context: FocusedContextState,
    pub search: FrontendSearchState,
    pub basic_operation: BasicOperationState,
    pub live_signals: LiveSignalsState,
    pub e2ee_trust: E2eeTrustState,
    pub local_encryption: LocalEncryptionState,
    pub native_attention: NativeAttentionState,
    pub cjk_text_policy: CjkTextPolicyState,
    pub errors: Vec<AppError>,
}

impl From<AppState> for FrontendAppState {
    fn from(state: AppState) -> Self {
        let platform = frontend_display_platform();
        let locale_profile =
            resolve_locale_display_profile(&state.settings.values.locale, platform);
        let typography_profile =
            resolve_typography_display_profile(&state.settings.values.typography, platform);
        Self {
            session: state.session.into(),
            auth: state.auth,
            settings: state.settings,
            locale_profile,
            typography_profile,
            profile: state.profile,
            sync: state.sync.into(),
            navigation: state.navigation,
            spaces: state.spaces,
            rooms: state.rooms,
            invites: state.invites,
            room_interactions: state.room_interactions,
            directory: state.directory,
            room_management: state.room_management,
            activity: state.activity,
            timeline: state.timeline,
            thread: state.thread.into(),
            focused_context: state.focused_context,
            search: state.search.into(),
            basic_operation: state.basic_operation,
            live_signals: state.live_signals,
            e2ee_trust: state.e2ee_trust,
            local_encryption: state.local_encryption,
            native_attention: state.native_attention,
            cjk_text_policy: state.cjk_text_policy,
            errors: state.errors,
        }
    }
}

fn frontend_display_platform() -> DisplayPlatform {
    #[cfg(target_os = "macos")]
    {
        DisplayPlatform::Macos
    }
    #[cfg(target_os = "windows")]
    {
        DisplayPlatform::Windows
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        DisplayPlatform::Linux
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FrontendSessionState {
    SignedOut,
    Restoring,
    SwitchingAccount {
        homeserver: String,
        user_id: String,
        device_id: String,
    },
    Authenticating {
        homeserver: String,
    },
    NeedsRecovery {
        homeserver: String,
        user_id: String,
        device_id: String,
        recovery_methods: Vec<RecoveryMethod>,
    },
    Recovering {
        homeserver: String,
        user_id: String,
        device_id: String,
        recovery_methods: Vec<RecoveryMethod>,
    },
    Ready {
        homeserver: String,
        user_id: String,
        device_id: String,
    },
    Locked {
        homeserver: String,
        user_id: String,
        device_id: String,
    },
    LoggingOut,
}

impl From<SessionState> for FrontendSessionState {
    fn from(session: SessionState) -> Self {
        match session {
            SessionState::SignedOut => Self::SignedOut,
            SessionState::Restoring => Self::Restoring,
            SessionState::SwitchingAccount { info } => Self::SwitchingAccount {
                homeserver: info.homeserver,
                user_id: info.user_id,
                device_id: info.device_id,
            },
            SessionState::Authenticating { homeserver } => Self::Authenticating { homeserver },
            SessionState::NeedsRecovery { info, methods } => Self::NeedsRecovery {
                homeserver: info.homeserver,
                user_id: info.user_id,
                device_id: info.device_id,
                recovery_methods: methods,
            },
            SessionState::Recovering { info, methods } => Self::Recovering {
                homeserver: info.homeserver,
                user_id: info.user_id,
                device_id: info.device_id,
                recovery_methods: methods,
            },
            SessionState::Ready(info) => Self::Ready {
                homeserver: info.homeserver,
                user_id: info.user_id,
                device_id: info.device_id,
            },
            SessionState::Locked(info) => Self::Locked {
                homeserver: info.homeserver,
                user_id: info.user_id,
                device_id: info.device_id,
            },
            SessionState::LoggingOut => Self::LoggingOut,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum FrontendSyncState {
    Name(&'static str),
    Failed { failed: String },
    Reconnecting { reconnecting: String },
}

impl From<SyncState> for FrontendSyncState {
    fn from(sync: SyncState) -> Self {
        match sync {
            SyncState::Stopped => Self::Name("stopped"),
            SyncState::Starting => Self::Name("starting"),
            SyncState::Running => Self::Name("running"),
            SyncState::Failed { reason } => Self::Failed { failed: reason },
            SyncState::Reconnecting { reason } => Self::Reconnecting {
                reconnecting: reason,
            },
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FrontendThreadPaneState {
    Closed,
    Opening {
        room_id: String,
        root_event_id: String,
    },
    Open {
        room_id: String,
        root_event_id: String,
        is_subscribed: bool,
        composer: ComposerState,
    },
}

impl From<ThreadPaneState> for FrontendThreadPaneState {
    fn from(thread: ThreadPaneState) -> Self {
        match thread {
            ThreadPaneState::Closed => Self::Closed,
            ThreadPaneState::Opening {
                room_id,
                root_event_id,
            } => Self::Opening {
                room_id,
                root_event_id,
            },
            ThreadPaneState::Open {
                room_id,
                root_event_id,
                is_subscribed,
                composer,
            } => Self::Open {
                room_id,
                root_event_id,
                is_subscribed,
                composer,
            },
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FrontendSearchState {
    Closed,
    Editing {
        query: String,
        scope: SearchScopeKind,
    },
    Searching {
        request_id: u64,
        query: String,
        scope: SearchScopeKind,
    },
    Results {
        request_id: u64,
        query: String,
        scope: SearchScopeKind,
        results: Vec<FrontendSearchResult>,
    },
    Failed {
        request_id: u64,
        query: String,
        scope: SearchScopeKind,
        message: String,
    },
}

impl From<SearchState> for FrontendSearchState {
    fn from(search: SearchState) -> Self {
        match search {
            SearchState::Closed => Self::Closed,
            SearchState::Editing { query, scope } => Self::Editing {
                query,
                scope: scope.into(),
            },
            SearchState::Searching {
                request_id,
                query,
                scope,
            } => Self::Searching {
                request_id,
                query,
                scope: scope.into(),
            },
            SearchState::Results {
                request_id,
                query,
                scope,
                results,
            } => Self::Results {
                request_id,
                query,
                scope: scope.into(),
                results: results.into_iter().map(Into::into).collect(),
            },
            SearchState::Failed {
                request_id,
                query,
                scope,
                message,
            } => Self::Failed {
                request_id,
                query,
                scope: scope.into(),
                message,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchScopeKind {
    CurrentRoom,
    CurrentSpace,
    Dms,
    AllRooms,
}

impl SearchScopeKind {
    #[allow(dead_code)]
    pub fn resolve(self, state: &AppState) -> SearchScope {
        match self {
            Self::CurrentRoom => state
                .navigation
                .active_room_id
                .as_ref()
                .map(|room_id| SearchScope::CurrentRoom {
                    room_id: room_id.clone(),
                })
                .unwrap_or(SearchScope::AllRooms),
            Self::CurrentSpace => state
                .navigation
                .active_space_id
                .as_ref()
                .map(|space_id| SearchScope::CurrentSpace {
                    space_id: space_id.clone(),
                })
                .unwrap_or(SearchScope::AllRooms),
            Self::Dms => SearchScope::Dms,
            Self::AllRooms => SearchScope::AllRooms,
        }
    }
}

impl From<SearchScope> for SearchScopeKind {
    fn from(scope: SearchScope) -> Self {
        match scope {
            SearchScope::CurrentRoom { .. } => Self::CurrentRoom,
            SearchScope::CurrentSpace { .. } => Self::CurrentSpace,
            SearchScope::Dms => Self::Dms,
            SearchScope::AllRooms => Self::AllRooms,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct FrontendSearchResult {
    pub room_id: String,
    pub event_id: String,
    pub sender: String,
    pub timestamp_ms: u64,
    pub score_millis: u32,
    pub snippet: String,
    pub match_field: FrontendSearchMatchField,
    pub highlights: Vec<matrix_desktop_state::TextRange>,
    pub match_kind: FrontendSearchMatchKind,
}

impl From<SearchResult> for FrontendSearchResult {
    fn from(result: SearchResult) -> Self {
        Self {
            room_id: result.room_id,
            event_id: result.event_id,
            sender: result.sender,
            timestamp_ms: result.timestamp_ms,
            score_millis: result.score_millis,
            snippet: result.snippet,
            match_field: result.match_field.into(),
            highlights: result.highlights,
            match_kind: result.match_kind.into(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FrontendSearchMatchField {
    MessageBody,
    AttachmentFileName,
}

impl From<SearchMatchField> for FrontendSearchMatchField {
    fn from(field: SearchMatchField) -> Self {
        match field {
            SearchMatchField::MessageBody => Self::MessageBody,
            SearchMatchField::AttachmentFileName => Self::AttachmentFileName,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FrontendSearchMatchKind {
    Exact,
}

impl From<SearchMatchKind> for FrontendSearchMatchKind {
    fn from(kind: SearchMatchKind) -> Self {
        match kind {
            SearchMatchKind::Exact => Self::Exact,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{FrontendDesktopSnapshot, FrontendSyncState};
    use matrix_desktop_state::{
        AppState, AvatarImage, AvatarThumbnailState, EmojiPreference, FontPreference,
        InvitePreview, LocaleSettings, OwnProfile, RecoveryMethod, RoomSummary, RoomTags,
        SessionInfo, SessionState, SpaceSummary, SyncState, TextDirectionPreference,
        TypographySettings, UserProfile,
    };

    fn booted_app_state() -> AppState {
        AppState {
            session: SessionState::Ready(SessionInfo {
                homeserver: "https://matrix.org".to_owned(),
                user_id: "@user:matrix.org".to_owned(),
                device_id: "DEVICE".to_owned(),
            }),
            sync: SyncState::Running,
            ..AppState::default()
        }
    }

    #[test]
    fn frontend_snapshot_serializes_to_the_typescript_contract() {
        let state = booted_app_state();
        let value = serde_json::to_value(FrontendDesktopSnapshot::from(state))
            .expect("snapshot should serialize");

        assert_eq!(value["state"]["session"]["kind"], json!("ready"));
        assert_eq!(
            value["state"]["session"]["homeserver"],
            json!("https://matrix.org")
        );
        assert_eq!(value["state"]["sync"], json!("running"));
        // invites must be present even when empty; React must not synthesize
        // invite state outside the Rust-owned state machine.
        assert_eq!(value["state"]["invites"], json!([]));
        // Core Batch A skeletons must be present in the real Tauri DTO, not
        // only in browser fakes.
        assert_eq!(value["state"]["room_interactions"], json!({}));
        assert_eq!(value["state"]["directory"]["kind"], json!("closed"));
        assert_eq!(
            value["state"]["room_management"]["selected_room_id"],
            json!(null)
        );
        assert_eq!(
            value["state"]["room_management"]["operation"]["kind"],
            json!("idle")
        );
        assert_eq!(value["state"]["activity"]["kind"], json!("closed"));
        // Phase 7: timeline is always [] (items flow as diffs)
        assert_eq!(value["timeline"], json!([]));
        // Phase 7: the legacy top-level thread is always null...
        assert_eq!(value["thread"], json!(null));
        // ...product thread state lives in state.thread (default Closed). The UI
        // reads the open/closed decision from here, not the legacy placeholder.
        assert_eq!(value["state"]["thread"]["kind"], json!("closed"));
        // focused_context must be present (default Closed) so the UI can drive
        // the focused search context view from the Rust-owned state machine.
        assert_eq!(value["state"]["focused_context"]["kind"], json!("closed"));
        // basic_operation must be present (default Idle) so the UI can read
        // snapshot.state.basic_operation.kind without crashing.
        assert_eq!(value["state"]["basic_operation"]["kind"], json!("idle"));
        // live_signals must be present so Phase B GUI renders Rust-owned live
        // signal state without inventing receipts, typing, or presence locally.
        assert_eq!(value["state"]["live_signals"]["rooms"], json!({}));
        assert_eq!(value["state"]["live_signals"]["presence"], json!({}));
        // e2ee_trust must be present (default private-data-free unknowns) so
        // later GUI work consumes the Rust-owned trust state machine.
        assert_eq!(
            value["state"]["e2ee_trust"]["verification"]["kind"],
            json!("idle")
        );
        assert_eq!(
            value["state"]["e2ee_trust"]["cross_signing"]["kind"],
            json!("unknown")
        );
        assert_eq!(
            value["state"]["e2ee_trust"]["key_backup"]["kind"],
            json!("unknown")
        );
        assert_eq!(
            value["state"]["e2ee_trust"]["identity_reset"]["kind"],
            json!("idle")
        );
        assert_eq!(
            value["state"]["local_encryption"]["kind"],
            json!("unknown")
        );
        assert_eq!(
            value["state"]["native_attention"]["dispatch"]["kind"],
            json!("idle")
        );
        assert_eq!(
            value["state"]["cjk_text_policy"]["japanese_catalog"]["catalog_locale"],
            json!("en")
        );
        assert_eq!(
            value["state"]["cjk_text_policy"]["normalization"]["form"],
            json!("nfkc")
        );
        assert_eq!(
            value["state"]["cjk_text_policy"]["collation"]["locale"],
            json!("ja")
        );
        // settings must be present so React can consume Rust-owned product
        // preferences instead of owning theme/locale/shortcut state.
        assert_eq!(
            value["state"]["settings"]["values"]["appearance"]["theme"],
            json!("system")
        );
        assert_eq!(
            value["state"]["settings"]["values"]["keyboard"]["composer_send_shortcut"],
            json!("enter")
        );
        assert_eq!(
            value["state"]["settings"]["persistence"]["kind"],
            json!("idle")
        );
        // locale_profile must be present so React applies root lang/dir and
        // catalog selection from Rust-owned settings/profile resolution.
        assert_eq!(value["state"]["locale_profile"]["lang"], json!("en"));
        assert_eq!(value["state"]["locale_profile"]["dir"], json!("ltr"));
        assert_eq!(
            value["state"]["locale_profile"]["catalog_locale"],
            json!("en")
        );
        assert_eq!(
            value["state"]["locale_profile"]["pseudo_locale"],
            json!("none")
        );
        // typography_profile must be present so React applies font and emoji
        // behavior from Rust-owned settings/profile resolution.
        assert_eq!(
            value["state"]["typography_profile"]["font"],
            json!("system")
        );
        assert_eq!(
            value["state"]["typography_profile"]["emoji"],
            json!("system")
        );
        assert_eq!(
            value["state"]["typography_profile"]["font_asset"],
            json!("systemFallback")
        );
        assert_eq!(
            value["state"]["typography_profile"]["emoji_asset"],
            json!("systemFallback")
        );
        // profile must be present so React displays and submits profile updates
        // from the Rust-owned profile state machine, never local component state.
        assert_eq!(
            value["state"]["profile"]["own"]["display_name"],
            json!(null)
        );
        assert_eq!(value["state"]["profile"]["own"]["avatar"], json!(null));
        assert_eq!(value["state"]["profile"]["users"], json!({}));
        assert_eq!(value["state"]["profile"]["update"]["kind"], json!("idle"));
        // composer.mode must be present (default Plain) for the same reason.
        assert_eq!(
            value["state"]["timeline"]["composer"]["mode"],
            json!("Plain")
        );
    }

    #[test]
    fn frontend_snapshot_serializes_invite_previews() {
        let mut state = booted_app_state();
        state.invites.push(InvitePreview {
            room_id: "!invite:matrix.org".to_owned(),
            display_name: "Project invite".to_owned(),
            avatar: None,
            topic: Some("Project topic".to_owned()),
            inviter_display_name: Some("Inviter".to_owned()),
            is_dm: true,
        });

        let value = serde_json::to_value(FrontendDesktopSnapshot::from(state))
            .expect("snapshot should serialize");

        assert_eq!(
            value["state"]["invites"],
            json!([
                {
                    "room_id": "!invite:matrix.org",
                    "display_name": "Project invite",
                    "avatar": null,
                    "topic": "Project topic",
                    "inviter_display_name": "Inviter",
                    "is_dm": true
                }
            ])
        );
    }

    #[test]
    fn frontend_snapshot_serializes_profile_and_summary_avatars() {
        let mut state = booted_app_state();
        let ready_avatar = AvatarImage {
            mxc_uri: "mxc://matrix.org/avatar".to_owned(),
            thumbnail: AvatarThumbnailState::Ready {
                source_url: "asset://avatar".to_owned(),
                width: Some(64),
                height: Some(64),
                mime_type: Some("image/png".to_owned()),
            },
        };
        let room_avatar = AvatarImage {
            mxc_uri: "mxc://matrix.org/room".to_owned(),
            thumbnail: AvatarThumbnailState::NotRequested,
        };
        state.profile.own = OwnProfile {
            display_name: Some("Alice".to_owned()),
            avatar: Some(ready_avatar.clone()),
        };
        state.profile.users.insert(
            "@bob:matrix.org".to_owned(),
            UserProfile {
                user_id: "@bob:matrix.org".to_owned(),
                display_name: Some("Bob".to_owned()),
                avatar: Some(ready_avatar),
            },
        );
        state.spaces.push(SpaceSummary {
            space_id: "!space:matrix.org".to_owned(),
            display_name: "Space".to_owned(),
            avatar: Some(room_avatar.clone()),
            child_room_ids: vec![],
        });
        state.rooms.push(RoomSummary {
            room_id: "!room:matrix.org".to_owned(),
            display_name: "Room".to_owned(),
            avatar: Some(room_avatar),
            is_dm: false,
            tags: RoomTags::default(),
            unread_count: 0,
            notification_count: 0,
            highlight_count: 0,
            parent_space_ids: vec![],
        });

        let value = serde_json::to_value(FrontendDesktopSnapshot::from(state))
            .expect("snapshot should serialize");

        assert_eq!(
            value["state"]["profile"]["own"],
            json!({
                "display_name": "Alice",
                "avatar": {
                    "mxc_uri": "mxc://matrix.org/avatar",
                    "thumbnail": {
                        "kind": "ready",
                        "source_url": "asset://avatar",
                        "width": 64,
                        "height": 64,
                        "mime_type": "image/png"
                    }
                }
            })
        );
        assert_eq!(
            value["state"]["profile"]["users"]["@bob:matrix.org"]["avatar"]["thumbnail"]["kind"],
            json!("ready")
        );
        assert_eq!(
            value["state"]["spaces"][0]["avatar"],
            json!({
                "mxc_uri": "mxc://matrix.org/room",
                "thumbnail": { "kind": "notRequested" }
            })
        );
        assert_eq!(
            value["state"]["rooms"][0]["avatar"],
            json!({
                "mxc_uri": "mxc://matrix.org/room",
                "thumbnail": { "kind": "notRequested" }
            })
        );
    }

    #[test]
    fn frontend_snapshot_locale_profile_follows_rust_owned_locale_settings() {
        let mut state = booted_app_state();
        state.settings.values.locale = LocaleSettings {
            language_tag: Some("ar-XB".to_owned()),
            text_direction: TextDirectionPreference::Auto,
        };

        let value = serde_json::to_value(FrontendDesktopSnapshot::from(state))
            .expect("snapshot should serialize");

        assert_eq!(value["state"]["locale_profile"]["lang"], json!("ar-XB"));
        assert_eq!(value["state"]["locale_profile"]["dir"], json!("rtl"));
        assert_eq!(
            value["state"]["locale_profile"]["catalog_locale"],
            json!("pseudo")
        );
        assert_eq!(
            value["state"]["locale_profile"]["pseudo_locale"],
            json!("bidi")
        );
        assert_ne!(
            value["state"]["locale_profile"]["modifier_labels"]["primary"],
            json!(null)
        );
    }

    #[test]
    fn frontend_snapshot_typography_profile_follows_rust_owned_typography_settings() {
        let mut state = booted_app_state();
        state.settings.values.typography = TypographySettings {
            font: FontPreference::Inter,
            emoji: EmojiPreference::TwemojiColr,
        };

        let value = serde_json::to_value(FrontendDesktopSnapshot::from(state))
            .expect("snapshot should serialize");

        assert_eq!(value["state"]["typography_profile"]["font"], json!("inter"));
        assert_eq!(
            value["state"]["typography_profile"]["emoji"],
            json!("twemojiColr")
        );
        assert_eq!(
            value["state"]["typography_profile"]["font_asset"],
            json!("bundledPreferred")
        );
        assert_eq!(
            value["state"]["typography_profile"]["emoji_asset"],
            json!("bundledPreferred")
        );
        assert_ne!(
            value["state"]["typography_profile"]["platform"],
            json!(null)
        );
    }

    #[test]
    fn frontend_snapshot_serializes_e2ee_recovery_step() {
        let state = AppState {
            session: SessionState::NeedsRecovery {
                info: SessionInfo {
                    homeserver: "https://matrix.org".to_owned(),
                    user_id: "@user:matrix.org".to_owned(),
                    device_id: "DEVICE".to_owned(),
                },
                methods: vec![RecoveryMethod::RecoveryKey, RecoveryMethod::SecurityPhrase],
            },
            sync: SyncState::Running,
            ..AppState::default()
        };

        let value = serde_json::to_value(FrontendDesktopSnapshot::from(state))
            .expect("snapshot should serialize");

        assert_eq!(value["state"]["session"]["kind"], json!("needsRecovery"));
        assert_eq!(
            value["state"]["session"]["recovery_methods"],
            json!(["recoveryKey", "securityPhrase"])
        );
        assert_eq!(value["state"]["sync"], json!("running"));
    }

    #[test]
    fn frontend_sync_state_serializes_failed_and_reconnecting() {
        assert_eq!(
            serde_json::to_value(FrontendSyncState::from(SyncState::Failed {
                reason: "limited network".to_owned(),
            }))
            .expect("failed sync should serialize"),
            json!({ "failed": "limited network" })
        );
        assert_eq!(
            serde_json::to_value(FrontendSyncState::from(SyncState::Reconnecting {
                reason: "limited network".to_owned(),
            }))
            .expect("reconnecting sync should serialize"),
            json!({ "reconnecting": "limited network" })
        );
    }
}
