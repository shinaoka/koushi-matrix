use serde_json::json;

use super::{
    FrontendCommandAdmission, FrontendCommandResult, FrontendCommandSettlement,
    FrontendDesktopSnapshot, FrontendDesktopSnapshotDelta, FrontendSyncState,
    frontend_display_platform,
};
use koushi_state::{
    AppState, AvatarImage, AvatarThumbnailState, EmojiPreference, FontPreference, InvitePreview,
    LocaleSettings, OwnProfile, RoomSummary, RoomTags, SessionInfo, SessionLockReason,
    SessionState, SpaceSummary, SyncState, TextDirectionPreference, TypographySettings,
    UserProfile, native_attention_capabilities_for_platform,
};

fn booted_app_state() -> AppState {
    AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://matrix.org".to_owned(),
            user_id: "@user:matrix.org".to_owned(),
            device_id: "DEVICE".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        sync: SyncState::Running,
        account_management_url: Some(koushi_state::AccountManagementUrl::from_validated(
            "https://account.example.test/devices".to_owned(),
        )),
        ..AppState::default()
    }
}

#[test]
fn frontend_snapshot_serializes_to_the_typescript_contract() {
    let state = booted_app_state();
    let value = serde_json::to_value(FrontendDesktopSnapshot::from(state))
        .expect("snapshot should serialize");

    assert_eq!(value["state"]["domain"]["session"]["kind"], json!("ready"));
    assert_eq!(
        value["state"]["domain"]["session"]["homeserver"],
        json!("https://matrix.org")
    );
    assert_eq!(value["state"]["domain"]["sync"], json!("running"));
    assert_eq!(
        value["state"]["domain"]["space_members"]["generation"],
        json!(0)
    );
    // invites must be present even when empty; React must not synthesize
    // invite state outside the Rust-owned state machine.
    assert_eq!(value["state"]["domain"]["invites"], json!([]));
    // Core Batch A skeletons must be present in the real Tauri DTO, not
    // only in browser fakes.
    assert_eq!(value["state"]["domain"]["room_interactions"], json!({}));
    assert_eq!(
        value["state"]["domain"]["account_management_url"],
        json!("https://account.example.test/devices")
    );
    assert_eq!(
        value["state"]["domain"]["device_cleanup"]["kind"],
        json!("idle")
    );
    assert_eq!(
        value["state"]["domain"]["account_management"]["kind"],
        json!("idle")
    );
    assert_eq!(
        value["state"]["domain"]["account_management_capabilities"]["change_password"]["kind"],
        json!("unknown")
    );
    assert_eq!(
        value["state"]["domain"]["soft_logout_reauth"]["kind"],
        json!("idle")
    );
    assert_eq!(value["state"]["domain"]["qr_login"]["kind"], json!("idle"));
    assert_eq!(
        value["state"]["domain"]["directory"]["query"]["kind"],
        json!("closed")
    );
    assert_eq!(
        value["state"]["domain"]["directory"]["join"]["kind"],
        json!("idle")
    );
    assert_eq!(
        value["state"]["domain"]["room_management"]["selected_room_id"],
        json!(null)
    );
    assert_eq!(
        value["state"]["domain"]["room_management"]["operation"]["kind"],
        json!("idle")
    );
    assert_eq!(
        value["state"]["domain"]["activity"]["kind"],
        json!("closed")
    );
    // Phase 7: timeline is always [] (items flow as diffs)
    assert_eq!(value["timeline"], json!([]));
    // Phase 7: the legacy top-level thread is always null...
    assert_eq!(value["thread"], json!(null));
    // ...product thread state lives in state.thread (default Closed). The UI
    // reads the open/closed decision from here, not the legacy placeholder.
    assert_eq!(value["state"]["ui"]["thread"]["kind"], json!("closed"));
    assert_eq!(
        value["state"]["domain"]["thread_attention"]["kind"],
        json!("closed")
    );
    // focused_context must be present (default Closed) so the UI can drive
    // the focused search context view from the Rust-owned state machine.
    assert_eq!(
        value["state"]["ui"]["focused_context"]["kind"],
        json!("closed")
    );
    // basic_operation must be present (default Idle) so the UI can read
    // snapshot.state.basic_operation.kind without crashing.
    assert_eq!(
        value["state"]["ui"]["basic_operation"]["kind"],
        json!("idle")
    );
    // room_list must be present so the UI renders the Rust-owned filtered
    // room-list projection instead of computing filters locally.
    assert_eq!(
        value["state"]["ui"]["room_list"]["active_filter"]["kind"],
        json!("rooms")
    );
    assert_eq!(value["state"]["ui"]["room_list"]["items"], json!([]));
    // live_signals must be present so Phase B GUI renders Rust-owned live
    // signal state without inventing receipts, typing, or presence locally.
    assert_eq!(value["state"]["domain"]["live_signals"]["rooms"], json!({}));
    assert_eq!(
        value["state"]["domain"]["live_signals"]["presence"],
        json!({})
    );
    // e2ee_trust must be present (default private-data-free unknowns) so
    // later GUI work consumes the Rust-owned trust state machine.
    assert_eq!(
        value["state"]["domain"]["e2ee_trust"]["verification"]["kind"],
        json!("idle")
    );
    assert_eq!(
        value["state"]["domain"]["e2ee_trust"]["cross_signing"]["kind"],
        json!("unknown")
    );
    assert_eq!(
        value["state"]["domain"]["e2ee_trust"]["key_backup"]["kind"],
        json!("unknown")
    );
    assert_eq!(
        value["state"]["domain"]["e2ee_trust"]["identity_reset"]["kind"],
        json!("idle")
    );
    assert_eq!(
        value["state"]["domain"]["e2ee_trust"]["key_management"]["room_key_export"]["kind"],
        json!("idle")
    );
    assert_eq!(
        value["state"]["domain"]["e2ee_trust"]["key_management"]["room_key_import"]["kind"],
        json!("idle")
    );
    assert_eq!(
        value["state"]["domain"]["e2ee_trust"]["key_management"]["secure_backup_setup"]["kind"],
        json!("idle")
    );
    assert_eq!(
        value["state"]["domain"]["e2ee_trust"]["key_management"]["passphrase_change"]["kind"],
        json!("idle")
    );
    assert_eq!(
        value["state"]["domain"]["local_encryption"]["kind"],
        json!("unknown")
    );
    assert_eq!(
        value["state"]["domain"]["native_attention"]["dispatch"]["kind"],
        json!("idle")
    );
    assert_eq!(
        value["state"]["domain"]["native_attention"]["summary"]["capabilities"],
        serde_json::to_value(native_attention_capabilities_for_platform(
            frontend_display_platform()
        ))
        .expect("capability profile serializes")
    );
    assert_eq!(
        value["state"]["domain"]["cjk_text_policy"]["japanese_catalog"]["catalog_locale"],
        json!("en")
    );
    assert_eq!(
        value["state"]["domain"]["cjk_text_policy"]["normalization"]["form"],
        json!("nfkc")
    );
    assert_eq!(
        value["state"]["domain"]["cjk_text_policy"]["collation"]["locale"],
        json!("ja")
    );
    // settings must be present so React can consume Rust-owned product
    // preferences instead of owning theme/locale/shortcut state.
    assert_eq!(
        value["state"]["domain"]["settings"]["values"]["appearance"]["theme"],
        json!("system")
    );
    assert_eq!(
        value["state"]["domain"]["settings"]["values"]["keyboard"]["composer_send_shortcut"],
        json!("enter")
    );
    assert_eq!(
        value["state"]["domain"]["settings"]["values"]["composer"],
        json!({ "math_mode": true, "recent_emojis": [] })
    );
    assert_eq!(
        value["state"]["domain"]["settings"]["values"]["appearance"]["density"],
        json!("comfortable")
    );
    assert_eq!(
        value["state"]["domain"]["settings"]["values"]["sidebar"]["category"],
        json!("rooms")
    );
    assert_eq!(
        value["state"]["domain"]["settings"]["values"]["notifications"],
        json!({
                "desktop_notifications": true,
                "sound": true,
                "badges": true,
                "send_read_receipts": true,
                "send_typing_notifications": true
        })
    );
    // room_notification_settings must be present (default empty) so the UI
    // renders per-room notification modes from Rust-owned state.
    assert_eq!(
        value["state"]["domain"]["room_notification_settings"],
        json!({})
    );
    // #305 retired the stored compression mode; only the encoder policy
    // crosses the boundary now.
    assert!(
        value["state"]["domain"]["settings"]["values"]["media"]["image_upload_compression"]
            .is_null(),
        "the retired compression mode must not reappear in the snapshot"
    );
    assert_eq!(
        value["state"]["domain"]["settings"]["values"]["media"]["image_upload_compression_policy"],
        json!({
                "threshold_bytes": 1048576,
                "threshold_long_edge": 2560,
                "target_long_edge": 2048,
                "quality_percent": 82
        })
    );
    assert_eq!(
        value["state"]["domain"]["settings"]["persistence"]["kind"],
        json!("idle")
    );
    // locale_profile must be present so React applies root lang/dir and
    // catalog selection from Rust-owned settings/profile resolution.
    assert_eq!(
        value["state"]["domain"]["locale_profile"]["lang"],
        json!("en")
    );
    assert_eq!(
        value["state"]["domain"]["locale_profile"]["dir"],
        json!("ltr")
    );
    assert_eq!(
        value["state"]["domain"]["locale_profile"]["catalog_locale"],
        json!("en")
    );
    assert_eq!(
        value["state"]["domain"]["locale_profile"]["pseudo_locale"],
        json!("none")
    );
    // typography_profile must be present so React applies font and emoji
    // behavior from Rust-owned settings/profile resolution.
    assert_eq!(
        value["state"]["domain"]["typography_profile"]["font"],
        json!("system")
    );
    assert_eq!(
        value["state"]["domain"]["typography_profile"]["emoji"],
        json!("system")
    );
    assert_eq!(
        value["state"]["domain"]["typography_profile"]["font_asset"],
        json!("systemFallback")
    );
    assert_eq!(
        value["state"]["domain"]["typography_profile"]["emoji_asset"],
        json!("systemFallback")
    );
    // profile must be present so React displays and submits profile updates
    // from the Rust-owned profile state machine, never local component state.
    assert_eq!(
        value["state"]["domain"]["profile"]["own"]["display_name"],
        json!(null)
    );
    assert_eq!(
        value["state"]["domain"]["profile"]["own"]["avatar"],
        json!(null)
    );
    assert_eq!(value["state"]["domain"]["profile"]["users"], json!({}));
    assert_eq!(
        value["state"]["domain"]["profile"]["update"]["kind"],
        json!("idle")
    );
    // composer.mode must be present (default Plain) for the same reason.
    assert_eq!(
        value["state"]["ui"]["timeline"]["composer"]["mode"],
        json!("Plain")
    );
    // The keyed draft backing store can contain non-visible unsent message
    // bodies. It stays Rust/core-internal; the webview receives only the
    // selected room/thread active composer.
    assert_eq!(value["state"]["domain"]["composer_drafts"], json!(null));
    // Scheduled-send backing state follows the same privacy boundary:
    // the full queue can contain future message bodies for non-visible
    // rooms, so only the selected timeline projection is serialized.
    assert_eq!(value["state"]["domain"]["scheduled_sends"], json!(null));
    // Upload staging and media-gallery backing stores follow the same
    // selected-room projection boundary. Hidden room filenames, captions,
    // and MXC URIs must not leak through the root AppState DTO.
    assert_eq!(value["state"]["domain"]["upload_staging"], json!(null));
    assert_eq!(value["state"]["domain"]["media_gallery"], json!(null));
    assert_eq!(
        value["state"]["ui"]["timeline"]["scheduled_send_capability"],
        json!("unknown")
    );
    assert_eq!(
        value["state"]["ui"]["timeline"]["scheduled_sends"],
        json!([])
    );
    assert_eq!(
        value["state"]["ui"]["timeline"]["staged_uploads"],
        json!([])
    );
    assert_eq!(value["state"]["ui"]["timeline"]["media_gallery"], json!([]));
}

#[test]
fn session_lock_reason_state_delta_crosses_the_frontend_boundary_and_clears_explicitly() {
    let previous = booted_app_state();
    let mut locked = previous.clone();
    locked.session_lock_reason = Some(SessionLockReason::UnknownToken { soft_logout: false });
    let delta = koushi_core::build_state_delta(9, &previous, &locked)
        .expect("session lock reason changes should produce a state delta");
    let value = serde_json::to_value(FrontendDesktopSnapshotDelta::from(delta))
        .expect("session lock reason delta should serialize");
    assert_eq!(
        value["changed"]["state"]["domain"]["session_lock_reason"],
        json!({"kind": "unknownToken", "soft_logout": false})
    );

    let clear = koushi_core::build_state_delta(10, &locked, &previous)
        .expect("clearing the lock reason should produce a state delta");
    let clear_value = serde_json::to_value(FrontendDesktopSnapshotDelta::from(clear))
        .expect("lock reason clear should serialize");
    assert_eq!(
        clear_value["changed"]["state"]["domain"]["session_lock_reason"],
        json!(null)
    );
}

#[test]
fn account_management_url_clear_crosses_the_frontend_boundary_as_null() {
    let previous = booted_app_state();
    let mut next = previous.clone();
    next.account_management_url = None;
    let delta = koushi_core::build_state_delta(10, &previous, &next)
        .expect("destination clear should produce a state delta");
    let value = serde_json::to_value(FrontendDesktopSnapshotDelta::from(delta))
        .expect("destination clear delta should serialize");

    assert_eq!(
        value["changed"]["state"]["domain"]["account_management_url"],
        json!(null)
    );
}

#[test]
fn space_member_state_delta_crosses_the_frontend_boundary() {
    let previous = booted_app_state();
    let mut next = previous.clone();
    next.space_members.selected_space_id = Some("!space:example.invalid".to_owned());
    next.space_members.generation = 4;

    let delta = koushi_core::build_state_delta(9, &previous, &next)
        .expect("Space member changes should produce a state delta");
    let value = serde_json::to_value(FrontendDesktopSnapshotDelta::from(delta))
        .expect("Space member delta should serialize");

    assert_eq!(
        value["changed"]["state"]["domain"]["space_members"]["generation"],
        json!(4)
    );
    assert_eq!(
        value["changed"]["state"]["domain"]["space_members"]["selected_space_id"],
        json!("!space:example.invalid")
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
        inviter_user_id: Some("@inviter:matrix.org".to_owned()),
        is_dm: true,
    });

    let value = serde_json::to_value(FrontendDesktopSnapshot::from(state))
        .expect("snapshot should serialize");

    assert_eq!(
        value["state"]["domain"]["invites"],
        json!([
            {
                    "room_id": "!invite:matrix.org",
                    "display_name": "Project invite",
                    "avatar": null,
                    "topic": "Project topic",
                    "inviter_display_name": "Inviter",
                    "inviter_user_id": "@inviter:matrix.org",
                    "is_dm": true
            }
        ])
    );
}

#[test]
fn frontend_snapshot_can_carry_state_delta_generation_for_reset_recovery() {
    let state = booted_app_state();
    let value = serde_json::to_value(FrontendDesktopSnapshot::from_versioned(state, 7))
        .expect("versioned snapshot should serialize");

    assert_eq!(value["state_generation"], json!(7));
    assert_eq!(value["state"]["domain"]["session"]["kind"], json!("ready"));
}

#[test]
fn frontend_snapshot_serializes_profile_and_summary_avatars() {
    let mut state = booted_app_state();
    let ready_avatar = AvatarImage {
        mxc_uri: "mxc://matrix.org/avatar".to_owned(),
        thumbnail: AvatarThumbnailState::Ready {
            source_ref: "avatar/1111111111111111".to_owned(),
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
            display_label: "Bob".to_owned(),
            original_display_label: "Bob".to_owned(),
            mention_search_terms: vec!["Bob".to_owned(), "@bob:matrix.org".to_owned()],
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
        display_label: "Room".to_owned(),
        original_display_label: "Room".to_owned(),
        avatar: Some(room_avatar),
        is_dm: false,
        dm_user_ids: Vec::new(),
        tags: RoomTags::default(),
        unread_count: 2,
        notification_count: 2,
        highlight_count: 1,
        marked_unread: false,
        recency_stamp: None,
        conversation_activity: None,
        latest_event: None,
        parent_space_ids: vec![],
        dm_space_ids: vec![],
        is_encrypted: false,
        joined_members: 0,
    });

    let value = serde_json::to_value(FrontendDesktopSnapshot::from(state))
        .expect("snapshot should serialize");

    assert_eq!(
        value["state"]["domain"]["profile"]["own"],
        json!({
                "display_name": "Alice",
                "avatar": {
                    "mxc_uri": "mxc://matrix.org/avatar",
                    "thumbnail": {
                        "kind": "ready",
                        "source_ref": "avatar/1111111111111111",
                        "width": 64,
                        "height": 64,
                        "mime_type": "image/png"
                }
            }
        })
    );
    assert_eq!(
        value["state"]["domain"]["profile"]["users"]["@bob:matrix.org"]["avatar"]["thumbnail"]["kind"],
        json!("ready")
    );
    assert_eq!(
        value["state"]["domain"]["profile"]["users"]["@bob:matrix.org"]["original_display_label"],
        json!("Bob")
    );
    assert_eq!(
        value["state"]["domain"]["spaces"][0]["avatar"],
        json!({
                "mxc_uri": "mxc://matrix.org/room",
                "thumbnail": { "kind": "notRequested" }
        })
    );
    assert_eq!(
        value["state"]["domain"]["rooms"][0]["avatar"],
        json!({
                "mxc_uri": "mxc://matrix.org/room",
                "thumbnail": { "kind": "notRequested" }
        })
    );
    assert_eq!(
        value["state"]["domain"]["rooms"][0]["original_display_label"],
        json!("Room")
    );
    assert_eq!(
        value["state"]["domain"]["rooms"][0]["dm_space_ids"],
        json!([])
    );
    assert_eq!(
        value["sidebar"]["account_home"]["highlight_count"],
        json!(1)
    );
    assert_eq!(
        value["sidebar"]["space_rooms"][0]["highlight_count"],
        json!(1)
    );
    assert_eq!(value["sidebar"]["space_highlight_count"], json!(1));
}

#[test]
fn frontend_snapshot_serializes_home_invite_and_attention_counts() {
    // #330: the Home rail badge renders `attention_count`, and its accessible
    // label names unread messages and invites separately, so all three have
    // to cross the Tauri boundary.
    let mut state = booted_app_state();
    for room_id in ["!invite-one:matrix.org", "!invite-two:matrix.org"] {
        state.invites.push(InvitePreview {
            room_id: room_id.to_owned(),
            display_name: "Invite".to_owned(),
            avatar: None,
            topic: None,
            inviter_display_name: None,
            inviter_user_id: None,
            is_dm: false,
        });
    }

    let value = serde_json::to_value(FrontendDesktopSnapshot::from(state))
        .expect("snapshot should serialize");
    let home = &value["sidebar"]["account_home"];
    let unread = home["unread_count"].as_u64().expect("unread count");

    assert_eq!(home["invite_count"], json!(2));
    assert_eq!(
        home["attention_count"],
        json!(unread + 2),
        "the badge total is unread messages plus pending invites"
    );
    assert_eq!(
        home["unread_count"],
        json!(unread),
        "invites must not be folded into the unread message count"
    );
}

#[test]
fn frontend_snapshot_sidebar_respects_muted_rooms_like_the_delta_path() {
    // The full-snapshot and delta transports must agree. Composing the
    // snapshot sidebar from rooms and spaces alone dropped mute filtering,
    // so the same state produced a different Home badge depending on which
    // transport delivered it.
    let mut state = booted_app_state();
    state.rooms.push(RoomSummary {
        room_id: "!muted:matrix.org".to_owned(),
        display_name: "Muted".to_owned(),
        display_label: "Muted".to_owned(),
        original_display_label: "Muted".to_owned(),
        avatar: None,
        is_dm: false,
        dm_user_ids: Vec::new(),
        tags: RoomTags::default(),
        unread_count: 4,
        notification_count: 4,
        highlight_count: 2,
        marked_unread: false,
        recency_stamp: None,
        conversation_activity: None,
        latest_event: None,
        parent_space_ids: vec![],
        dm_space_ids: vec![],
        is_encrypted: false,
        joined_members: 0,
    });
    state.room_notification_settings.insert(
        "!muted:matrix.org".to_owned(),
        koushi_state::RoomNotificationSettings {
            mode: koushi_state::RoomNotificationMode::Mute,
            ..koushi_state::RoomNotificationSettings::default()
        },
    );

    let value = serde_json::to_value(FrontendDesktopSnapshot::from(state))
        .expect("snapshot should serialize");

    assert_eq!(
        value["sidebar"]["account_home"]["unread_count"],
        json!(0),
        "a muted room must not raise the Home unread count"
    );
    assert_eq!(
        value["sidebar"]["account_home"]["highlight_count"],
        json!(0),
        "a muted room must not raise the Home highlight count"
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

    assert_eq!(
        value["state"]["domain"]["locale_profile"]["lang"],
        json!("ar-XB")
    );
    assert_eq!(
        value["state"]["domain"]["locale_profile"]["dir"],
        json!("rtl")
    );
    assert_eq!(
        value["state"]["domain"]["locale_profile"]["catalog_locale"],
        json!("pseudo")
    );
    assert_eq!(
        value["state"]["domain"]["locale_profile"]["pseudo_locale"],
        json!("bidi")
    );
    assert_ne!(
        value["state"]["domain"]["locale_profile"]["modifier_labels"]["primary"],
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

    assert_eq!(
        value["state"]["domain"]["typography_profile"]["font"],
        json!("inter")
    );
    assert_eq!(
        value["state"]["domain"]["typography_profile"]["emoji"],
        json!("twemojiColr")
    );
    assert_eq!(
        value["state"]["domain"]["typography_profile"]["font_asset"],
        json!("bundledPreferred")
    );
    assert_eq!(
        value["state"]["domain"]["typography_profile"]["emoji_asset"],
        json!("bundledPreferred")
    );
    assert_ne!(
        value["state"]["domain"]["typography_profile"]["platform"],
        json!(null)
    );
}

#[test]
fn frontend_snapshot_serializes_verification_gate() {
    let state = AppState {
        session: SessionState::AwaitingVerification {
            info: SessionInfo {
                homeserver: "https://matrix.org".to_owned(),
                user_id: "@user:matrix.org".to_owned(),
                device_id: "DEVICE".to_owned(),
                authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
            },
            gate: koushi_state::VerificationGateState {
                methods: vec![
                    koushi_state::VerificationMethodCapability::RecoveryKey,
                    koushi_state::VerificationMethodCapability::SecurityPhrase,
                ],
                account_kind: koushi_state::VerificationAccountKind::ExistingIdentity,
                failure: None,
            },
        },
        sync: SyncState::Running,
        ..AppState::default()
    };

    let value = serde_json::to_value(FrontendDesktopSnapshot::from(state))
        .expect("snapshot should serialize");

    assert_eq!(
        value["state"]["domain"]["session"]["kind"],
        json!("awaitingVerification")
    );
    assert_eq!(
        value["state"]["domain"]["session"]["gate"]["methods"],
        json!(["recoveryKey", "securityPhrase"])
    );
    assert_eq!(value["state"]["domain"]["sync"], json!("running"));
}

#[test]
fn frontend_session_gate_variants_are_private_safe_json() {
    let info = SessionInfo {
        homeserver: "https://example.invalid".into(),
        user_id: "@private:example.invalid".into(),
        device_id: "PRIVATEDEVICE".into(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    };
    let gate = koushi_state::VerificationGateState {
        methods: vec![
            koushi_state::VerificationMethodCapability::ExistingDeviceSas,
            koushi_state::VerificationMethodCapability::Bootstrap,
        ],
        account_kind: koushi_state::VerificationAccountKind::ExistingIdentity,
        failure: Some(koushi_state::VerificationGateFailureKind::Network),
    };
    let sessions = [
        SessionState::Provisional {
            info: info.clone(),
            phase: koushi_state::ProvisionalPhase::RecheckingTrust {
                failure: Some(koushi_state::VerificationGateFailureKind::Timeout),
            },
        },
        SessionState::AwaitingVerification {
            info: info.clone(),
            gate: gate.clone(),
        },
        SessionState::Verifying {
            info: info.clone(),
            gate: gate.clone(),
            method: koushi_state::VerificationMethod::ExistingDeviceSas,
            flow_id: 7,
            sas_emojis: vec![
                koushi_state::SasEmoji {
                    symbol: "🐶".into(),
                    description: "dog".into()
                };
                7
            ],
        },
        SessionState::AwaitingBootstrapConfirmation {
            info: info.clone(),
            gate: gate.clone(),
            flow_id: 8,
            destination_written: true,
        },
        SessionState::Rejecting {
            info,
            reason: koushi_state::VerificationGateRejectReason::UserRejected,
        },
    ];
    let expected = [
        "provisional",
        "awaitingVerification",
        "verifying",
        "awaitingBootstrapConfirmation",
        "rejecting",
    ];
    for (session, kind) in sessions.into_iter().zip(expected) {
        let value =
            serde_json::to_value(super::FrontendSessionState::from(session)).expect("gate DTO");
        assert_eq!(value["kind"], kind);
        let wire = value.to_string();
        assert!(!wire.contains("secret"));
        assert!(!wire.contains("destination_path"));
        assert!(!wire.contains("target_user"));
    }
}

#[test]
fn frontend_capability_blocked_session_serializes_typed_failures() {
    let info = SessionInfo {
        homeserver: "https://example.invalid".into(),
        user_id: "@blocked:example.invalid".into(),
        device_id: "BLOCKEDDEVICE".into(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    };

    for (failure, expected) in [
        (
            koushi_state::SlidingSyncCapabilityFailureKind::Unsupported,
            "unsupported",
        ),
        (
            koushi_state::SlidingSyncCapabilityFailureKind::Unreachable,
            "unreachable",
        ),
        (
            koushi_state::SlidingSyncCapabilityFailureKind::InvalidResponse,
            "invalidResponse",
        ),
    ] {
        let value = serde_json::to_value(super::FrontendSessionState::from(
            SessionState::CapabilityBlocked {
                info: info.clone(),
                failure,
            },
        ))
        .expect("capability-blocked session should serialize");
        assert_eq!(value["kind"], "capabilityBlocked");
        assert_eq!(value["failure"], expected);
    }
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

#[test]
fn composer_revision_tauri_wire_round_trips_max_and_rejects_numeric() {
    use koushi_state::{ComposerDraftRevision, ComposerState, ThreadPaneState, TimelinePaneState};

    let mut state = booted_app_state();
    state.timeline = TimelinePaneState {
        composer: ComposerState {
            draft_revision: ComposerDraftRevision::MAX,
            last_accepted_clear_revision: ComposerDraftRevision::MAX,
            ..ComposerState::default()
        },
        ..TimelinePaneState::default()
    };
    state.thread = ThreadPaneState::Open {
        room_id: "!room:example.invalid".to_owned(),
        root_event_id: "$root:example.invalid".to_owned(),
        intent: koushi_state::ThreadOpenIntent::ExistingThread,
        is_subscribed: true,
        composer: ComposerState {
            draft_revision: ComposerDraftRevision::MAX,
            last_accepted_clear_revision: ComposerDraftRevision::MAX,
            ..ComposerState::default()
        },
        staged_uploads: Vec::new(),
    };

    let value = serde_json::to_value(FrontendDesktopSnapshot::from(state))
        .expect("max composer revisions should serialize");
    let max = json!("340282366920938463463374607431768211455");
    assert_eq!(
        value["state"]["ui"]["timeline"]["composer"]["draft_revision"],
        max
    );
    assert_eq!(
        value["state"]["ui"]["timeline"]["composer"]["last_accepted_clear_revision"],
        max
    );
    assert_eq!(
        value["state"]["ui"]["thread"]["composer"]["draft_revision"],
        max
    );
    assert_eq!(
        value["state"]["ui"]["thread"]["composer"]["last_accepted_clear_revision"],
        max
    );

    assert!(
        serde_json::from_value::<ComposerDraftRevision>(json!(
            "340282366920938463463374607431768211455"
        ))
        .is_ok()
    );
    assert!(serde_json::from_value::<ComposerDraftRevision>(json!(1)).is_err());
}

/// Characterization / golden test for the complete `FrontendAppState` DTO wire shape.
///
/// Purpose: lock in the exact JSON serialization of `FrontendDesktopSnapshot` so any
/// later Phase 2 (file splits) or Phase 4 (domain/ui DTO reorg) that silently drops,
/// renames, or reorders a field is caught immediately.
///
/// The golden artifact lives at `tests/golden/frontend_app_state.json`.
///
/// To regenerate: `UPDATE_GOLDEN=1 cargo test -p koushi-desktop frontend_app_state_golden_matches_maximally_populated_state`
///
/// When to regenerate: ONLY after an intentional, reviewed DTO change (Phase 4 etc.).
/// A failing golden test with no intentional change signals an accidental field loss —
/// investigate before regenerating.
#[test]
fn frontend_app_state_golden_matches_maximally_populated_state() {
    use koushi_state::{
        ActivityMarkReadState, ActivityRow, ActivityState, ActivityStream, ActivityTab,
        AttachmentFilter, AttachmentKind, AttachmentResult, AttachmentScope, AttachmentSort,
        AvatarImage, AvatarThumbnailState, BasicOperationState, CrossSigningStatus,
        DeviceCleanupFailureKind, DeviceCleanupLocalMode, DeviceCleanupRemoteOutcome,
        DeviceCleanupState, DirectoryJoinState, DirectoryPreviewJoinability,
        DirectoryPreviewMembership, DirectoryPreviewState, DirectoryQuery, DirectoryQueryState,
        DirectoryRoomPreview, DirectoryRoomSummary, DirectoryState, E2eeKeyManagementState,
        E2eeTrustState, FilesViewState, FocusedContextState, IdentityResetState, InvitePreview,
        KeyBackupStatus, LiveSignalsState, LocalEncryptionState, MediaTransferProgress,
        NativeAttentionCandidate, NativeAttentionCapabilities, NativeAttentionCapability,
        NativeAttentionDispatchState, NativeAttentionState, NativeAttentionSummary,
        NavigationState, OwnProfile, PinOp, PinOperationState, PinnedEvent, RoomAttentionKind,
        RoomHistoryVisibility, RoomInteractionState, RoomJoinRule, RoomLatestEventSummary,
        RoomLiveSignals, RoomManagementOperationState, RoomManagementState, RoomMemberRole,
        RoomMemberSummary, RoomNotificationSettings, RoomPermissionFacts, RoomSettingsSnapshot,
        RoomSummary, RoomTags, SearchMatchField, SearchMatchKind, SearchResult, SearchScope,
        SearchState, SessionInfo, SessionState, SpaceSummary, SyncState, TextRange,
        ThreadAttentionState, ThreadsListItem, ThreadsListState, TimelineMediaDownloadState,
        TimelinePaneState, UserProfile, VerificationFlowState,
    };
    use std::collections::BTreeMap;

    // Construct a maximally-populated AppState. Every section gets at least one
    // non-default field so that Phase 2/4 refactors cannot silently drop it.
    // All identifiers are synthetic (example.invalid / fixture pattern).
    let session_info = SessionInfo {
        homeserver: "https://matrix.example.invalid".to_owned(),
        user_id: "@fixture:example.invalid".to_owned(),
        device_id: "FIXTURE_DEVICE".to_owned(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    };
    let avatar = AvatarImage {
        mxc_uri: "mxc://example.invalid/fixture-avatar".to_owned(),
        thumbnail: AvatarThumbnailState::Ready {
            source_ref: "avatar/2222222222222222".to_owned(),
            width: Some(64),
            height: Some(64),
            mime_type: Some("image/png".to_owned()),
        },
    };

    let mut state = AppState {
        session: SessionState::Ready(session_info.clone()),
        account_management_url: Some(koushi_state::AccountManagementUrl::from_validated(
            "https://account.example.invalid/devices".to_owned(),
        )),
        device_cleanup: DeviceCleanupState::LocalResetFailed {
            request_id: 370,
            mode: DeviceCleanupLocalMode::RemoteRemoved {
                outcome: DeviceCleanupRemoteOutcome::AlreadyAbsent,
            },
            failure: DeviceCleanupFailureKind::LocalData,
        },
        sync: SyncState::Running,
        ..AppState::default()
    };
    state.current_session_status = koushi_state::CurrentSessionStatusState::Ready {
        request_id: 369,
        details: koushi_state::CurrentSessionStatusDetails::new(
            Some("Fixture Device".to_owned()),
            session_info.device_id.clone(),
            koushi_state::SessionAuthenticationMethod::OAuth,
            koushi_state::CurrentSessionSyncState::Running,
            koushi_state::CurrentDeviceTrustState::Verified,
            true,
            koushi_state::OwnIdentityVerification::Verified,
            koushi_state::CurrentSessionBackupState::Ready,
            1_722_000_000_000,
        ),
    };

    // profile — own + one cached user
    state.profile.own = OwnProfile {
        display_name: Some("Fixture User".to_owned()),
        avatar: Some(avatar.clone()),
    };
    state.profile.users.insert(
        "@other:example.invalid".to_owned(),
        UserProfile {
            user_id: "@other:example.invalid".to_owned(),
            display_name: Some("Other Fixture".to_owned()),
            display_label: "Other Fixture".to_owned(),
            original_display_label: "Other Fixture".to_owned(),
            mention_search_terms: vec!["other".to_owned()],
            avatar: Some(avatar.clone()),
        },
    );

    // spaces + rooms
    state.spaces.push(SpaceSummary {
        space_id: "!space:example.invalid".to_owned(),
        display_name: "Fixture Space".to_owned(),
        avatar: None,
        child_room_ids: vec!["!room:example.invalid".to_owned()],
    });
    state.rooms.push(RoomSummary {
        room_id: "!room:example.invalid".to_owned(),
        display_name: "Fixture Room".to_owned(),
        display_label: "Fixture Room".to_owned(),
        original_display_label: "Fixture Room".to_owned(),
        avatar: Some(avatar.clone()),
        is_dm: false,
        dm_user_ids: Vec::new(),
        tags: RoomTags::default(),
        unread_count: 3,
        notification_count: 2,
        highlight_count: 1,
        marked_unread: false,
        recency_stamp: Some(1_000_000),
        conversation_activity: None,
        latest_event: Some(RoomLatestEventSummary {
            event_id: "$fixture-latest:example.invalid".to_owned(),
            relation_type: None,
            relation_event_id: None,
            sender_id: Some("@other:example.invalid".to_owned()),
            sender_label: Some("Other Fixture".to_owned()),
            sender_avatar: None,
            preview: Some("Fixture latest message".to_owned()),
            timestamp_ms: 1_000_000,
            is_redacted: false,
        }),
        parent_space_ids: vec!["!space:example.invalid".to_owned()],
        dm_space_ids: vec![],
        is_encrypted: true,
        joined_members: 4,
    });
    state.rooms.push(RoomSummary {
        room_id: "!redacted-room:example.invalid".to_owned(),
        display_name: "Redacted Fixture Room".to_owned(),
        display_label: "Redacted Fixture Room".to_owned(),
        original_display_label: "Redacted Fixture Room".to_owned(),
        avatar: None,
        is_dm: false,
        dm_user_ids: Vec::new(),
        tags: RoomTags::default(),
        unread_count: 0,
        notification_count: 0,
        highlight_count: 0,
        marked_unread: false,
        recency_stamp: Some(900_000),
        conversation_activity: None,
        latest_event: Some(RoomLatestEventSummary {
            event_id: "$fixture-redacted:example.invalid".to_owned(),
            relation_type: None,
            relation_event_id: None,
            sender_id: Some("@other:example.invalid".to_owned()),
            sender_label: Some("Other Fixture".to_owned()),
            sender_avatar: None,
            preview: None,
            timestamp_ms: 900_000,
            is_redacted: true,
        }),
        parent_space_ids: vec![],
        dm_space_ids: vec![],
        is_encrypted: false,
        joined_members: 2,
    });
    state.room_list.readiness = koushi_state::RoomListReadiness::Ready {
        source: koushi_state::RoomListSource::Live,
        generation: 9,
    };

    // invites
    state.invites.push(InvitePreview {
        room_id: "!invite:example.invalid".to_owned(),
        display_name: "Fixture Invite".to_owned(),
        avatar: None,
        topic: Some("Fixture invite topic".to_owned()),
        inviter_display_name: Some("Inviter".to_owned()),
        inviter_user_id: Some("@inviter:example.invalid".to_owned()),
        is_dm: false,
    });

    // navigation — active room + space
    state.navigation = NavigationState {
        active_room_id: Some("!room:example.invalid".to_owned()),
        active_space_id: Some("!space:example.invalid".to_owned()),
        home_selection: koushi_state::HomeSelection::DirectMessage {
            room_id: "!dm:example.invalid".to_owned(),
        },
        space_local_presentations: koushi_state::SpaceLocalPresentations(BTreeMap::from([(
            "!space:example.invalid".to_owned(),
            koushi_state::SpaceLocalPresentation {
                name: Some("Local Space".to_owned()),
                icon: Some("L".to_owned()),
            },
        )])),
        legacy_frontend_preferences_imported: true,
        space_order: vec!["!space:example.invalid".to_owned()],
        last_room_by_space_id: BTreeMap::from([(
            "!space:example.invalid".to_owned(),
            "!room:example.invalid".to_owned(),
        )]),
        // #445: exercise the real shape — a DMs-surface selection, which an
        // empty map or a `None` room id would not prove.
        last_selection_by_space_id: BTreeMap::from([(
            "!space:example.invalid".to_owned(),
            koushi_state::SpaceNavigationSelection {
                surface: koushi_state::SpaceConversationSurface::Dms,
                room_id: Some("!dm:example.invalid".to_owned()),
            },
        )]),
        room_scroll_anchors: BTreeMap::new(),
        main_timeline_anchor: None,
        event_navigation: koushi_state::EventNavigationState::Idle,
    };

    // room_interactions
    state.room_interactions.insert(
        "!room:example.invalid".to_owned(),
        RoomInteractionState {
            pinned_events: vec![PinnedEvent {
                event_id: "$pinned:example.invalid".to_owned(),
                sender: Some("@fixture:example.invalid".to_owned()),
                sender_label: Some("Fixture User".to_owned()),
                body_preview: Some("Pinned fixture message".to_owned()),
                redacted: false,
                timestamp_ms: Some(1_800_000_000_000),
                state: koushi_state::PinnedEventState::Ready,
                thread_root_event_id: None,
            }],
            pin_operation: PinOperationState::Pending {
                request_id: 42,
                room_id: "!room:example.invalid".to_owned(),
                event_id: "$pinned:example.invalid".to_owned(),
                op: PinOp::Pin,
            },
        },
    );

    // room_notification_settings
    state.room_notification_settings.insert(
        "!room:example.invalid".to_owned(),
        RoomNotificationSettings::default(),
    );

    // room_management — with settings snapshot
    state.room_management = RoomManagementState {
        selected_room_id: Some("!room:example.invalid".to_owned()),
        settings: Some(RoomSettingsSnapshot {
            room_id: "!room:example.invalid".to_owned(),
            name: Some("Fixture Room".to_owned()),
            topic: Some("Fixture room topic".to_owned()),
            avatar_url: Some("mxc://example.invalid/room-avatar".to_owned()),
            canonical_alias: None,
            alternate_aliases: Vec::new(),
            share_link: None,
            join_rule: RoomJoinRule::Invite,
            history_visibility: RoomHistoryVisibility::Shared,
            permissions: RoomPermissionFacts {
                can_edit_settings: true,
                can_edit_roles: true,
                can_invite: true,
                can_kick: true,
                can_ban: true,
                can_unban: true,
            },
            members: vec![RoomMemberSummary {
                user_id: "@fixture:example.invalid".to_owned(),
                display_name: Some("Fixture User".to_owned()),
                display_label: "Fixture User".to_owned(),
                original_display_label: "Fixture User".to_owned(),
                avatar_url: None,
                power_level: Some(100),
                role: RoomMemberRole::Administrator,
                role_options: Vec::new(),
                user_trust: None,
            }],
        }),
        operation: RoomManagementOperationState::Idle,
    };

    // mention_candidates — partial main and complete thread targets
    state.mention_candidates = koushi_state::MentionCandidatesState {
        targets: vec![
            koushi_state::MentionCandidatesTarget {
                room_id: "!room:example.invalid".to_owned(),
                generation: 7,
                request_id: 70,
                query: "fi".to_owned(),
                surface: koushi_state::MentionSurface::Main,
                completeness: koushi_state::MentionCandidatesCompleteness::Partial,
                candidates: vec![koushi_state::MentionCandidate {
                    user_id: "@fixture:example.invalid".to_owned(),
                    display_label: Some("Fixture User".to_owned()),
                    original_display_label: Some("Fixture User".to_owned()),
                    avatar: Some(koushi_state::AvatarImage {
                        mxc_uri: "mxc://example.invalid/mention-avatar".to_owned(),
                        thumbnail: koushi_state::AvatarThumbnailState::NotRequested,
                    }),
                    membership: koushi_state::MentionCandidateMembership::Joined,
                }],
                room_mention_allowed: koushi_state::RoomMentionPermission::Allowed,
                failure_kind: None,
            },
            koushi_state::MentionCandidatesTarget {
                room_id: "!room:example.invalid".to_owned(),
                generation: 8,
                request_id: 71,
                query: String::new(),
                surface: koushi_state::MentionSurface::Thread,
                completeness: koushi_state::MentionCandidatesCompleteness::Complete,
                candidates: vec![koushi_state::MentionCandidate {
                    user_id: "@unlabelled:example.invalid".to_owned(),
                    display_label: None,
                    original_display_label: None,
                    avatar: None,
                    membership: koushi_state::MentionCandidateMembership::Joined,
                }],
                room_mention_allowed: koushi_state::RoomMentionPermission::Denied,
                failure_kind: None,
            },
        ],
    };

    // activity — open with populated streams
    state.activity = ActivityState::Open {
        active_tab: ActivityTab::Recent,
        recent: ActivityStream {
            rows: vec![ActivityRow {
                kind: koushi_state::ActivityRowKind::Event,
                room_id: "!room:example.invalid".to_owned(),
                event_id: Some("$act:example.invalid".to_owned()),
                room_label: "Fixture Room".to_owned(),
                sender_label: Some("Fixture User".to_owned()),
                preview: Some("Activity preview".to_owned()),
                timestamp_ms: 500_000,
                unread: false,
                highlight: false,
                ..Default::default()
            }],
            next_batch: None,
            resolution: Default::default(),
        },
        unread: ActivityStream {
            rows: vec![ActivityRow::room_unread_placeholder(
                "!placeholder:example.invalid".to_owned(),
                "Placeholder Room".to_owned(),
                499_000,
                true,
            )],
            next_batch: None,
            resolution: Default::default(),
        },
        mark_read: ActivityMarkReadState::Idle,
    };

    // timeline — composer + media_downloads populated
    let mut composer = koushi_state::ComposerState::default();
    composer
        .accepted_submission_ids
        .push_back(koushi_state::SubmissionId::new("accepted-contract"));
    composer.document = koushi_state::ComposerDocument::new(vec![
        koushi_state::ComposerInline::Text {
            text: "Hello ".to_owned(),
        },
        koushi_state::ComposerInline::Mention {
            target: koushi_state::MentionTarget::User {
                user_id: "@fixture:example.invalid".to_owned(),
                display_label: "Fixture User".to_owned(),
            },
            display_label: "Fixture User".to_owned(),
        },
    ]);
    composer.draft = composer.document.plain_body();
    composer.draft_revision = koushi_state::ComposerDraftRevision::MAX;
    composer.last_accepted_clear_revision = koushi_state::ComposerDraftRevision::MAX;
    state.timeline = TimelinePaneState {
        room_id: Some("!room:example.invalid".to_owned()),
        is_subscribed: true,
        is_paginating_backwards: false,
        composer,
        submission_registry: koushi_state::ComposerSubmissionRegistry {
            accepted_submission_ids: [koushi_state::SubmissionId::new("global-accepted")]
                .into_iter()
                .collect(),
            settled_submission_ids: [koushi_state::SubmissionId::new("global-settled")]
                .into_iter()
                .collect(),
            active_submissions: Default::default(),
        },
        scheduled_send_capability: Default::default(),
        scheduled_sends: Vec::new(),
        staged_uploads: Vec::new(),
        media_gallery: Vec::new(),
        media_downloads: {
            let mut m = BTreeMap::new();
            m.insert(
                "$media:example.invalid".to_owned(),
                TimelineMediaDownloadState::Pending {
                    progress: Some(MediaTransferProgress {
                        current: 10,
                        total: 100,
                    }),
                },
            );
            m
        },
        continuity: koushi_state::TimelineContinuityState::FailedIncomplete {
            generation: 7,
            gap_count: 2,
            batches_processed: 3,
            failure_kind: koushi_state::TimelineGapRepairFailureKind::Sdk,
        },
    };
    state.thread = koushi_state::ThreadPaneState::Open {
        room_id: "!room:example.invalid".to_owned(),
        root_event_id: "$thread-root:example.invalid".to_owned(),
        intent: koushi_state::ThreadOpenIntent::NewThreadDraft,
        is_subscribed: true,
        composer: koushi_state::ComposerState {
            draft: "Thread @room".to_owned(),
            document: koushi_state::ComposerDocument::new(vec![
                koushi_state::ComposerInline::Text {
                    text: "Thread ".to_owned(),
                },
                koushi_state::ComposerInline::Mention {
                    target: koushi_state::MentionTarget::RoomMention {
                        display_label: "room".to_owned(),
                    },
                    display_label: "room".to_owned(),
                },
            ]),
            draft_revision: koushi_state::ComposerDraftRevision::MAX,
            last_accepted_clear_revision: koushi_state::ComposerDraftRevision::MAX,
            ..koushi_state::ComposerState::default()
        },
        staged_uploads: Vec::new(),
    };

    // live_signals — one room entry
    state.live_signals = LiveSignalsState {
        rooms: {
            let mut m = BTreeMap::new();
            m.insert(
                "!room:example.invalid".to_owned(),
                RoomLiveSignals {
                    receipts_by_event: BTreeMap::new(),
                    fully_read_event_id: Some("$read:example.invalid".to_owned()),
                    typing_user_ids: vec!["@other:example.invalid".to_owned()],
                    typing_users: vec![koushi_state::LiveTypingUser {
                        user_id: "@other:example.invalid".to_owned(),
                        display_label: Some("Other Person".to_owned()),
                    }],
                },
            );
            m
        },
        presence: {
            let mut m = BTreeMap::new();
            m.insert(
                "@fixture:example.invalid".to_owned(),
                koushi_state::PresenceKind::Online,
            );
            m
        },
    };

    // e2ee_trust — non-default fields
    state.e2ee_trust = E2eeTrustState {
        verification: VerificationFlowState::Idle,
        cross_signing: CrossSigningStatus::Trusted,
        key_backup: KeyBackupStatus::Enabled {
            version: "v1".to_owned(),
        },
        identity_reset: IdentityResetState::Idle,
        key_management: E2eeKeyManagementState::default(),
        devices: Vec::new(),
    };

    // local_encryption
    state.local_encryption = LocalEncryptionState::Healthy;

    // native_attention — non-default capabilities
    state.native_attention = NativeAttentionState {
        summary: NativeAttentionSummary {
            unread_count: 5,
            highlight_count: 2,
            badge_count: 5,
            candidate: Some(NativeAttentionCandidate {
                room_display_name: "Fixture Room".to_owned(),
                kind: RoomAttentionKind::Message,
                unread_count: 1,
                highlight_count: 1,
            }),
            capabilities: NativeAttentionCapabilities {
                notifications: NativeAttentionCapability::Available,
                badge: NativeAttentionCapability::Available,
                overlay_icon: NativeAttentionCapability::Unavailable,
                sound: NativeAttentionCapability::Available,
                tray: NativeAttentionCapability::Unknown,
                activation: NativeAttentionCapability::Available,
            },
        },
        dispatch: NativeAttentionDispatchState::Idle,
    };

    // directory — Results with one entry + Joining join state
    state.directory = DirectoryState {
        query: DirectoryQueryState::Results {
            request_id: 7,
            query: DirectoryQuery {
                term: Some("fixture".to_owned()),
                server_name: None,
                limit: Some(20),
                since: None,
            },
            rooms: vec![DirectoryRoomSummary {
                room_id: "!dir:example.invalid".to_owned(),
                canonical_alias: Some("#fixture:example.invalid".to_owned()),
                room_type: Some("m.space".to_owned()),
                name: "Fixture Public Room".to_owned(),
                topic: Some("Fixture topic".to_owned()),
                avatar_url: None,
                joined_members: 42,
                world_readable: true,
                guest_can_join: false,
            }],
            next_batch: None,
        },
        // Ready, so the golden pins the whole preview payload rather than
        // a variant that carries none of its fields.
        preview: DirectoryPreviewState::Ready {
            request_id: 9,
            room_id_or_alias: "!fixture-preview:example.invalid".to_owned(),
            via_servers: vec!["preview.example.invalid".to_owned()],
            room: DirectoryRoomPreview {
                room_id: "!fixture-preview:example.invalid".to_owned(),
                canonical_alias: Some("#fixture-preview:example.invalid".to_owned()),
                room_type: Some("m.space".to_owned()),
                name: "Fixture Previewed Space".to_owned(),
                topic: Some("Fixture preview topic".to_owned()),
                joined_members: 12,
                joinability: DirectoryPreviewJoinability::Restricted,
                membership: DirectoryPreviewMembership::Invited,
            },
        },
        join: DirectoryJoinState::Joining {
            request_id: 8,
            room_id_or_alias: "#fixture:example.invalid".to_owned(),
            // Two servers so the golden covers the list shape, not just
            // an empty array that any scalar field would also satisfy.
            via_servers: vec![
                "first.example.invalid".to_owned(),
                "second.example.invalid".to_owned(),
            ],
        },
    };

    // focused_context — Open referencing a synthetic event
    state.focused_context = FocusedContextState::Open {
        room_id: "!room:example.invalid".to_owned(),
        event_id: "$focused:example.invalid".to_owned(),
        is_subscribed: true,
    };

    // search — Results with one entry
    state.search = SearchState::Results {
        request_id: 9,
        query: "fixture query".to_owned(),
        scope: SearchScope::AllRooms,
        results: vec![SearchResult {
            room_id: "!room:example.invalid".to_owned(),
            event_id: "$search:example.invalid".to_owned(),
            context_label: Some("Fixture Space · Fixture Room".to_owned()),
            sender: "@fixture:example.invalid".to_owned(),
            timestamp_ms: 600_000,
            score_millis: 950,
            snippet: "Fixture search snippet".to_owned(),
            match_field: SearchMatchField::MessageBody,
            highlights: vec![TextRange {
                start_utf16: 8,
                end_utf16: 14,
            }],
            match_kind: SearchMatchKind::Exact,
        }],
    };

    // files_view — Open with one attachment entry
    state.files_view = FilesViewState::Open {
        request_id: 10,
        scope: AttachmentScope::Room {
            room_id: "!room:example.invalid".to_owned(),
        },
        filter: AttachmentFilter {
            kinds: vec![AttachmentKind::Image],
            filename_query: None,
        },
        sort: AttachmentSort::NewestFirst,
        items: vec![AttachmentResult {
            event_id: "$attach:example.invalid".to_owned(),
            filename: "fixture.png".to_owned(),
            kind: AttachmentKind::Image,
            mimetype: Some("image/png".to_owned()),
            room_id: "!room:example.invalid".to_owned(),
            sender: "@fixture:example.invalid".to_owned(),
            sender_label: Some("Fixture User".to_owned()),
            size: Some(1024),
            source_mxc: "mxc://example.invalid/attach".to_owned(),
            thumbnail_mxc: None,
            timestamp_ms: 700_000,
            thread_root: None,
            encrypted: false,
            encryption_version: None,
            width: Some(128),
            height: Some(128),
            is_edited: false,
        }],
        selected_event_id: Some("$attach:example.invalid".to_owned()),
    };

    // threads_list — Open with one thread row
    state.threads_list = ThreadsListState::Open {
        room_id: "!room:example.invalid".to_owned(),
        request_id: 11,
        items: vec![ThreadsListItem {
            room_id: "!room:example.invalid".to_owned(),
            root_event_id: "$thread-root:example.invalid".to_owned(),
            root_sender: "@fixture:example.invalid".to_owned(),
            root_sender_label: Some("Fixture User".to_owned()),
            root_body_preview: Some("Thread root preview".to_owned()),
            root_timestamp_ms: Some(800_000),
            latest_event_id: Some("$thread-reply:example.invalid".to_owned()),
            latest_sender: Some("@other:example.invalid".to_owned()),
            latest_sender_label: Some("Other Fixture".to_owned()),
            latest_body_preview: Some("Latest reply preview".to_owned()),
            latest_timestamp_ms: Some(810_000),
            reply_count: 3,
        }],
        is_paginating: false,
        end_reached: false,
    };

    // thread_attention — Tracking with non-zero counts
    state.thread_attention = ThreadAttentionState::Tracking {
        room_id: "!room:example.invalid".to_owned(),
        root_event_id: "$thread-root:example.invalid".to_owned(),
        notification_count: 4,
        highlight_count: 1,
        live_event_marker_count: 2,
    };

    // basic_operation — non-default (creating room)
    state.basic_operation = BasicOperationState::CreatingRoom {
        request_id: 1,
        name: "Fixture New Room".to_owned(),
    };

    // Serialize
    let sidebar = koushi_state::compose_sidebar_for_state(&state);
    let value = serde_json::to_value(FrontendDesktopSnapshot {
        state_generation: None,
        state: super::frontend_app_state_for_platform(state, koushi_state::DisplayPlatform::Linux),
        sidebar,
        timeline: Vec::new(),
        thread: None,
    })
    .expect("maximally-populated state should serialize to JSON");
    assert_eq!(
        value["state"]["domain"]["current_session_status"]["status"], "ready",
        "the complete Rust-owned current-session status must cross the Tauri DTO"
    );

    let golden_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/golden/frontend_app_state.json"
    );

    if std::env::var("UPDATE_GOLDEN").as_deref() == Ok("1") {
        let pretty = serde_json::to_string_pretty(&value).expect("should format golden JSON");
        std::fs::create_dir_all(std::path::Path::new(golden_path).parent().unwrap())
            .expect("golden directory should be creatable");
        std::fs::write(golden_path, pretty).expect("golden artifact should be writable");
        return;
    }

    let golden_bytes = std::fs::read(golden_path).unwrap_or_else(|_| {
        panic!(
            "golden artifact not found at {golden_path}. \
                Run with UPDATE_GOLDEN=1 to generate it."
        )
    });
    let golden: serde_json::Value =
        serde_json::from_slice(&golden_bytes).expect("golden artifact must be valid JSON");

    assert_eq!(
        value, golden,
        "FrontendAppState wire shape changed — if intentional, regenerate with UPDATE_GOLDEN=1"
    );
}

#[test]
fn state_update_envelope_serializes_the_v1_delta_and_snapshot_shapes() {
    use super::{FrontendStateUpdateEnvelope, StateUpdateSnapshotReason};
    use koushi_protocol::state_update::VersionedAppStateSnapshot;

    let delta = FrontendStateUpdateEnvelope::delta(FrontendDesktopSnapshotDelta {
        generation: 7,
        changed: Default::default(),
    });
    assert_eq!(
        serde_json::to_value(delta).expect("delta envelope should serialize"),
        json!({
            "protocol_version": 1,
            "kind": "delta",
            "generation": 7,
            "changed": {}
        })
    );

    let snapshot = FrontendStateUpdateEnvelope::snapshot(
        VersionedAppStateSnapshot {
            generation: 11,
            state: AppState::default(),
        },
        StateUpdateSnapshotReason::Gap,
    );
    let value = serde_json::to_value(snapshot).expect("snapshot envelope should serialize");
    assert_eq!(value["protocol_version"], json!(1));
    assert_eq!(value["kind"], json!("snapshot"));
    assert_eq!(value["generation"], json!(11));
    assert_eq!(value["reason"], json!("gap"));
    assert_eq!(value["snapshot"]["state_generation"], json!(11));
}

#[test]
fn command_admission_serializes_as_v1_camel_case_dto() {
    let value = serde_json::to_value(FrontendCommandAdmission {
        protocol_version: 1,
        admitted_generation: 42,
    })
    .expect("command admission should serialize");

    assert_eq!(
        value,
        json!({
            "protocolVersion": 1,
            "admittedGeneration": 42,
        })
    );
}

#[test]
fn command_result_nests_the_typed_result_and_v1_settlement() {
    let value = serde_json::to_value(FrontendCommandResult::new(
        "accepted",
        FrontendCommandSettlement::from_published_generation(43),
    ))
    .expect("command result should serialize");

    assert_eq!(
        value,
        json!({
            "result": "accepted",
            "settlement": {
                "protocolVersion": 1,
                "publishedGeneration": 43,
            },
        })
    );
}

#[test]
fn command_settlement_serializes_as_v1_camel_case_dto() {
    let value = serde_json::to_value(FrontendCommandSettlement {
        protocol_version: 1,
        published_generation: 43,
    })
    .expect("command settlement should serialize");

    assert_eq!(
        value,
        json!({
            "protocolVersion": 1,
            "publishedGeneration": 43,
        })
    );
}
