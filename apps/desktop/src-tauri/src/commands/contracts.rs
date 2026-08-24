use super::*;
use super::{
    activity::*, directory::*, e2ee::*, live_signals::*, navigation::*, profile::*, room::*,
    search::*, session::*, timeline::*, views::*,
};
use koushi_core::AccountKey;
use koushi_core::{
    AccountCommand, AppCommand, CoreCommand, CoreConnection, CoreEvent, CreateRoomOptions,
    CreateRoomParentSpace, CreateRoomVisibility, ImageUploadCompressionPolicy,
    ImageUploadCompressionState, ImageUploadDimensions, ImageUploadVariantInfo,
    ImageUploadVariantKind, IntentNoOpReason, IntentOutcome, MediaDownloadSelection,
    PaginationDirection, RequestId, RoomCommand, SearchCommand, SearchScope, SyncCommand,
    TimelineCommand, TimelineKey, UploadMediaKind, UploadMediaThumbnail,
};
use koushi_state::{ActivityMarkReadTarget, ActivityTab, ImageUploadCompressionMode};
use koushi_state::{
    AppState, AuthSecret, ComposerDocument, DisplayPlatform, LoginRequest, PresenceKind,
    RoomHistoryVisibility, RoomJoinRule, RoomModerationAction, RoomSettingChange, RoomTagKind,
    ThreadOpenIntent,
};
use std::collections::VecDeque;

fn production_file(source: &str) -> &str {
    source
        .split("\n#[cfg(test)]\nmod ")
        .next()
        .unwrap_or(source)
}
pub(super) fn production_source() -> String {
    [
        include_str!("session.rs"),
        include_str!("settings.rs"),
        include_str!("account.rs"),
        include_str!("local_encryption.rs"),
        include_str!("e2ee.rs"),
        include_str!("navigation.rs"),
        include_str!("timeline.rs"),
        include_str!("live_signals.rs"),
        include_str!("profile.rs"),
        include_str!("directory.rs"),
        include_str!("room.rs"),
        include_str!("activity.rs"),
        include_str!("views.rs"),
        include_str!("search.rs"),
        include_str!("mod.rs"),
    ]
    .into_iter()
    .map(production_file)
    .collect()
}

pub(super) struct ScriptedSelectSource {
    pub(super) snapshot: AppState,
    pub(super) events: VecDeque<Result<CoreEvent, koushi_core::EventStreamLag>>,
}

struct ScriptedSearchPathIo;

const SYNTHETIC_QUERY: &str = "  synthetic-query-text event synthetic-event-id user synthetic-user-id body synthetic-body-text url https://synthetic.example/path absolute /synthetic/private/path  ";

pub(super) fn fake_request_id(sequence: u64) -> koushi_core::RequestId {
    koushi_core::RequestId {
        connection_id: koushi_core::RuntimeConnectionId(7),
        sequence,
    }
}

pub(super) fn synthetic_session_key() -> koushi_key::SessionKeyId {
    koushi_key::SessionKeyId {
        homeserver: "https://example.org".to_owned(),
        user_id: "@alice:example.org".to_owned(),
        device_id: "DEVICE".to_owned(),
    }
}

impl super::search::SearchPathIo for ScriptedSearchPathIo {
    fn submit<'a>(
        &'a self,
        _state: &'a super::CoreRuntimeState,
        command: CoreCommand,
    ) -> super::search::SearchPathFuture<'a> {
        match command {
            CoreCommand::Search(SearchCommand::Query { query, scope, .. }) => {
                assert_eq!(query, SYNTHETIC_QUERY);
                assert_eq!(
                    scope,
                    SearchScope::CurrentRoom {
                        room_id: "synthetic-room-id".to_owned()
                    }
                );
            }
            other => panic!("unexpected search command: {other:?}"),
        }
        Box::pin(std::future::ready(Ok(())))
    }

    fn wait<'a>(
        &'a self,
        _connection: &'a mut CoreConnection,
        _request_id: RequestId,
    ) -> super::search::SearchPathFuture<'a> {
        Box::pin(std::future::ready(Ok(())))
    }
}

impl super::navigation::SelectEventSource for ScriptedSelectSource {
    fn snapshot(&self) -> AppState {
        self.snapshot.clone()
    }

    fn recv_event(
        &mut self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<CoreEvent, koushi_core::EventStreamLag>>
                + Send
                + '_,
        >,
    > {
        Box::pin(std::future::ready(self.events.pop_front().unwrap_or_else(
            || Err(koushi_core::EventStreamLag { skipped: 0 }),
        )))
    }
}

#[test]
fn submit_core_command_does_not_hold_connection_mutex_while_awaiting_send() {
    let source = production_source();
    let production_source = source
        .split("#[cfg(test)]\nmod tests")
        .next()
        .expect("command production source should precede tests");
    let helper_source = production_source
        .split("pub(crate) async fn submit_core_command")
        .nth(1)
        .expect("submit_core_command helper should exist")
        .split("/// Allocate a `RequestId`")
        .next()
        .expect("next helper should follow submit_core_command");

    assert!(
        production_source.contains("const CORE_COMMAND_SUBMIT_TIMEOUT"),
        "Tauri command submits should have a bounded wait instead of blocking snapshots indefinitely"
    );
    assert!(
        helper_source.contains("command_handle"),
        "submit_core_command should clone a lightweight submit handle before awaiting send"
    );
    assert!(
        helper_source.contains("tokio::time::timeout(CORE_COMMAND_SUBMIT_TIMEOUT"),
        "submit_core_command should bound backpressured command sends"
    );
    assert!(
        !helper_source
            .contains(".lock()\n        .await\n        .command(command)\n        .await")
            && !helper_source.contains(".lock().await.command(command).await"),
        "submit_core_command must not hold the shared CoreConnection mutex across send().await"
    );
}

#[test]
fn event_wait_loops_resync_on_lag_instead_of_failing_immediately() {
    let source = production_source();
    let waiters = [
        (
            "async fn wait_for_logged_in_authenticated",
            "async fn wait_for_auth_changed",
        ),
        (
            "async fn wait_for_auth_changed",
            "fn snapshot_has_authenticated_session",
        ),
        (
            "async fn wait_for_focused_context_closed",
            "async fn wait_for_focused_context",
        ),
        (
            "async fn wait_for_focused_context",
            "async fn wait_for_main_timeline_anchor",
        ),
        (
            "async fn wait_for_main_timeline_anchor",
            "async fn wait_for_selected_room",
        ),
        (
            "async fn wait_for_selected_room",
            "fn snapshot_has_active_room",
        ),
        (
            "async fn wait_for_search_started",
            "async fn wait_for_search_closed",
        ),
        (
            "async fn wait_for_search_closed",
            "fn select_active_room_trace_label",
        ),
        (
            "async fn wait_for_upload_staging_snapshot",
            "pub struct StageUploadInputItem",
        ),
        (
            "async fn wait_for_room_created",
            "async fn wait_for_space_created",
        ),
        (
            "async fn wait_for_space_created",
            "async fn wait_for_room_operation",
        ),
        (
            "async fn wait_for_room_operation",
            "async fn wait_for_room_joined",
        ),
        ("async fn wait_for_room_joined", "pub async fn create_room"),
        (
            "async fn wait_for_invite_batch_completed",
            "pub async fn invite_users",
        ),
        (
            "async fn wait_for_oidc_authorization",
            "#[tauri::command]\npub async fn submit_login",
        ),
    ];

    for (start, end) in waiters {
        let body = source
            .split(start)
            .nth(1)
            .unwrap_or_else(|| panic!("{start} should exist"))
            .split(end)
            .next()
            .unwrap_or_else(|| panic!("{end} should follow {start}"));
        assert!(
            !body.contains("event stream lagged"),
            "{start} should re-check snapshot or keep waiting after EventStreamLag"
        );
    }
}

#[test]
fn correlated_operation_failures_preserve_core_failure_kind_in_invoke_errors() {
    let source = production_source();
    let failure_waiters = [
        (
            "async fn wait_for_logged_in_authenticated",
            "async fn wait_for_auth_changed",
        ),
        (
            "async fn wait_for_focused_context_closed",
            "async fn wait_for_focused_context",
        ),
        (
            "async fn wait_for_focused_context",
            "async fn wait_for_main_timeline_anchor",
        ),
        (
            "async fn wait_for_main_timeline_anchor",
            "async fn wait_for_selected_room",
        ),
        (
            "async fn wait_for_search_started",
            "async fn wait_for_search_closed",
        ),
        (
            "async fn wait_for_search_closed",
            "fn select_active_room_trace_label",
        ),
        (
            "async fn wait_for_upload_staging_snapshot",
            "pub struct StageUploadInputItem",
        ),
        (
            "async fn wait_for_room_created",
            "async fn wait_for_space_created",
        ),
        (
            "async fn wait_for_space_created",
            "async fn wait_for_room_operation",
        ),
        (
            "async fn wait_for_room_operation",
            "async fn wait_for_room_joined",
        ),
        ("async fn wait_for_room_joined", "pub async fn create_room"),
        (
            "async fn wait_for_invite_batch_completed",
            "pub async fn invite_users",
        ),
        (
            "async fn wait_for_oidc_authorization",
            "#[tauri::command]\npub async fn submit_login",
        ),
        (
            "pub async fn list_saved_sessions",
            "#[tauri::command]\npub async fn switch_account",
        ),
    ];

    for (start, end) in failure_waiters {
        let body = source
            .split(start)
            .nth(1)
            .unwrap_or_else(|| panic!("{start} should exist"))
            .split(end)
            .next()
            .unwrap_or_else(|| panic!("{end} should follow {start}"));
        assert!(
            body.contains("invoke_error_from_core_failure"),
            "{start} should include the typed CoreFailure kind in invoke errors"
        );
    }
}

#[test]
fn tauri_command_routes_build_expected_core_commands() {
    let active_account_key = AccountKey("@alice:example.org".to_owned());
    let active_session_key = koushi_key::SessionKeyId {
        homeserver: "https://example.org".to_owned(),
        user_id: "@alice:example.org".to_owned(),
        device_id: "DEVICE".to_owned(),
    };
    let room_id = "!room:example.org".to_owned();
    let transaction_id = "desktop-1".to_owned();
    let body = "body with visible content".to_owned();
    let edit_body = "updated body".to_owned();
    let query = "search terms".to_owned();

    match build_submit_login_command(
        fake_request_id(1),
        LoginRequest {
            homeserver: "https://matrix.example.org".to_owned(),
            username: "alice".to_owned(),
            password: AuthSecret::new("password-123"),
            device_display_name: Some("Laptop".to_owned()),
        },
        DisplayPlatform::Linux,
    ) {
        CoreCommand::Account(AccountCommand::LoginPassword {
            request_id,
            request,
            platform,
        }) => {
            assert_eq!(request_id, fake_request_id(1));
            assert_eq!(request.homeserver, "https://matrix.example.org");
            assert_eq!(request.username, "alice");
            assert_eq!(request.password.expose_secret(), "password-123");
            assert_eq!(request.device_display_name.as_deref(), Some("Laptop"));
            assert_eq!(platform, DisplayPlatform::Linux);
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_submit_soft_logout_reauth_command(
        fake_request_id(102),
        AuthSecret::new("reauth-password-123"),
    ) {
        CoreCommand::Account(AccountCommand::SoftLogoutReauth {
            request_id,
            password,
        }) => {
            assert_eq!(request_id, fake_request_id(102));
            assert_eq!(password.expose_secret(), "reauth-password-123");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_discover_login_command(
        fake_request_id(101),
        "https://matrix.example.org".to_owned(),
    ) {
        CoreCommand::Account(AccountCommand::DiscoverLogin {
            request_id,
            homeserver,
        }) => {
            assert_eq!(request_id, fake_request_id(101));
            assert_eq!(homeserver, "https://matrix.example.org");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_switch_account_command(fake_request_id(2), "@bob:example.org".to_owned()) {
        CoreCommand::Account(AccountCommand::SwitchAccount {
            request_id,
            account_key,
        }) => {
            assert_eq!(request_id, fake_request_id(2));
            assert_eq!(
                account_key,
                koushi_core::AccountKey("@bob:example.org".to_owned())
            );
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_submit_recovery_command(fake_request_id(3), AuthSecret::new("recovery-123")) {
        CoreCommand::Account(AccountCommand::SubmitRecovery {
            request_id,
            request,
        }) => {
            assert_eq!(request_id, fake_request_id(3));
            assert_eq!(request.secret.expose_secret(), "recovery-123");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_export_room_keys_command(
        fake_request_id(33),
        "/tmp/element-compatible-export.txt".to_owned(),
        AuthSecret::new("room-key-transfer-phrase"),
    ) {
        CoreCommand::Account(AccountCommand::ExportRoomKeys {
            request_id,
            request,
        }) => {
            assert_eq!(request_id, fake_request_id(33));
            assert_eq!(
                request.destination_path,
                std::path::PathBuf::from("/tmp/element-compatible-export.txt")
            );
            assert_eq!(
                request.passphrase.expose_secret(),
                "room-key-transfer-phrase"
            );
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_import_room_keys_command(
        fake_request_id(34),
        "/tmp/element-compatible-import.txt".to_owned(),
        AuthSecret::new("room-key-transfer-phrase"),
    ) {
        CoreCommand::Account(AccountCommand::ImportRoomKeys {
            request_id,
            request,
        }) => {
            assert_eq!(request_id, fake_request_id(34));
            assert_eq!(
                request.source_path,
                std::path::PathBuf::from("/tmp/element-compatible-import.txt")
            );
            assert_eq!(
                request.passphrase.expose_secret(),
                "room-key-transfer-phrase"
            );
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_bootstrap_secure_backup_command(
        fake_request_id(35),
        Some(AuthSecret::new("backup-setup-phrase")),
        Some("/tmp/recovery-artifact.txt".to_owned()),
        false,
    ) {
        CoreCommand::Account(AccountCommand::BootstrapSecureBackup {
            request_id,
            request,
        }) => {
            assert_eq!(request_id, fake_request_id(35));
            assert_eq!(
                request
                    .passphrase
                    .as_ref()
                    .expect("passphrase")
                    .expose_secret(),
                "backup-setup-phrase"
            );
            assert_eq!(
                request.recovery_key_destination_path,
                Some(std::path::PathBuf::from("/tmp/recovery-artifact.txt"))
            );
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_change_secure_backup_passphrase_command(
        fake_request_id(36),
        AuthSecret::new("old-backup-phrase"),
        AuthSecret::new("new-backup-phrase"),
        Some("/tmp/recovery-artifact.txt".to_owned()),
    ) {
        CoreCommand::Account(AccountCommand::ChangeSecureBackupPassphrase {
            request_id,
            request,
        }) => {
            assert_eq!(request_id, fake_request_id(36));
            assert_eq!(request.old_secret.expose_secret(), "old-backup-phrase");
            assert_eq!(request.new_passphrase.expose_secret(), "new-backup-phrase");
            assert_eq!(
                request.recovery_key_destination_path,
                Some(std::path::PathBuf::from("/tmp/recovery-artifact.txt"))
            );
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_logout_command(fake_request_id(4)) {
        CoreCommand::Account(AccountCommand::Logout { request_id }) => {
            assert_eq!(request_id, fake_request_id(4));
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_retry_sliding_sync_capability_command(fake_request_id(41)) {
        CoreCommand::Account(AccountCommand::RetrySlidingSyncCapability { request_id }) => {
            assert_eq!(request_id, fake_request_id(41));
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_change_homeserver_command(fake_request_id(42)) {
        CoreCommand::Account(AccountCommand::ChangeHomeserver { request_id }) => {
            assert_eq!(request_id, fake_request_id(42));
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_restart_sync_command(fake_request_id(5)) {
        CoreCommand::Sync(SyncCommand::Restart { request_id }) => {
            assert_eq!(request_id, fake_request_id(5));
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_select_space_command(fake_request_id(6), Some("!space:example.org".to_owned())) {
        CoreCommand::Room(RoomCommand::SelectSpace {
            request_id,
            space_id,
        }) => {
            assert_eq!(request_id, fake_request_id(6));
            assert_eq!(space_id.as_deref(), Some("!space:example.org"));
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_reorder_spaces_command(
        fake_request_id(37),
        vec![
            "!space-b:example.org".to_owned(),
            "!space-a:example.org".to_owned(),
        ],
    ) {
        CoreCommand::Room(RoomCommand::ReorderSpaces {
            request_id,
            space_ids,
        }) => {
            assert_eq!(request_id, fake_request_id(37));
            assert_eq!(
                space_ids,
                vec!["!space-b:example.org", "!space-a:example.org"]
            );
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_select_room_command(fake_request_id(7), room_id.clone()) {
        CoreCommand::Room(RoomCommand::SelectRoom {
            request_id,
            room_id: route_room_id,
        }) => {
            assert_eq!(request_id, fake_request_id(7));
            assert_eq!(route_room_id, room_id);
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_paginate_timeline_backwards_command(
        fake_request_id(9),
        active_account_key.clone(),
        room_id.clone(),
    ) {
        CoreCommand::Timeline(TimelineCommand::Paginate {
            request_id,
            key,
            direction,
            event_count,
        }) => {
            assert_eq!(request_id, fake_request_id(9));
            assert_eq!(key.account_key, active_account_key);
            assert_eq!(
                key.kind,
                koushi_core::TimelineKind::Room {
                    room_id: room_id.clone()
                }
            );
            assert_eq!(direction, PaginationDirection::Backward);
            assert_eq!(event_count, 100);
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_restore_timeline_anchor_command(
        fake_request_id(10),
        active_account_key.clone(),
        koushi_core::TimelineKey::room(active_account_key.clone(), room_id.clone()),
        "$anchor:example.invalid".to_owned(),
        TIMELINE_RESTORE_ANCHOR_MAX_BATCHES,
        TIMELINE_BACKWARDS_PAGE_EVENT_COUNT,
    ) {
        CoreCommand::Timeline(TimelineCommand::RestoreTimelineAnchor {
            request_id,
            key,
            event_id,
            max_batches,
            event_count,
        }) => {
            assert_eq!(request_id, fake_request_id(10));
            assert_eq!(key.account_key, active_account_key);
            assert_eq!(
                key.kind,
                koushi_core::TimelineKind::Room {
                    room_id: room_id.clone()
                }
            );
            assert_eq!(event_id, "$anchor:example.invalid");
            assert_eq!(max_batches, TIMELINE_RESTORE_ANCHOR_MAX_BATCHES);
            assert_eq!(event_count, TIMELINE_BACKWARDS_PAGE_EVENT_COUNT);
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_send_text_command(
        fake_request_id(11),
        active_account_key.clone(),
        room_id.clone(),
        transaction_id.clone(),
        ComposerDocument::from_plain_text(body.clone()),
    )
    .expect("send_text should build a command")
    {
        CoreCommand::Timeline(TimelineCommand::SendText {
            request_id,
            key,
            transaction_id: route_transaction_id,
            document,
        }) => {
            assert_eq!(request_id, fake_request_id(11));
            assert_eq!(key.account_key, active_account_key);
            assert_eq!(
                key.kind,
                koushi_core::TimelineKind::Room {
                    room_id: room_id.clone()
                }
            );
            assert_eq!(route_transaction_id, transaction_id);
            assert_eq!(document.plain_body(), body);
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_schedule_send_command(
        fake_request_id(33),
        active_session_key.clone(),
        koushi_state::ComposerTarget::Main {
            room_id: room_id.clone(),
        },
        "send later body".to_owned(),
        1_900_000_000_000,
        7.into(),
    )
    .expect("schedule_send should build a command")
    {
        CoreCommand::App(AppCommand::ScheduleSend {
            request_id,
            expected_account,
            room_id: route_room_id,
            thread_root_event_id,
            body,
            send_at_ms,
            draft_revision,
        }) => {
            assert_eq!(request_id, fake_request_id(33));
            assert_eq!(expected_account, active_session_key);
            assert_eq!(route_room_id, room_id);
            assert_eq!(thread_root_event_id, None);
            assert_eq!(body, "send later body");
            assert_eq!(send_at_ms, 1_900_000_000_000);
            assert_eq!(draft_revision, 7.into());
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_schedule_send_command(
        fake_request_id(330),
        active_session_key.clone(),
        koushi_state::ComposerTarget::Thread {
            room_id: room_id.clone(),
            root_event_id: "$thread-root:example.test".to_owned(),
        },
        "thread later body".to_owned(),
        1_900_000_010_000,
        8.into(),
    )
    .expect("thread schedule_send should build a command")
    {
        CoreCommand::App(AppCommand::ScheduleSend {
            room_id: route_room_id,
            thread_root_event_id,
            ..
        }) => {
            assert_eq!(route_room_id, room_id);
            assert_eq!(
                thread_root_event_id.as_deref(),
                Some("$thread-root:example.test")
            );
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_cancel_scheduled_send_command(fake_request_id(34), "scheduled-1".to_owned())
        .expect("cancel_scheduled_send should build a command")
    {
        CoreCommand::App(AppCommand::CancelScheduledSend {
            request_id,
            scheduled_id,
        }) => {
            assert_eq!(request_id, fake_request_id(34));
            assert_eq!(scheduled_id, "scheduled-1");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_reschedule_scheduled_send_command(
        fake_request_id(35),
        "scheduled-1".to_owned(),
        "edited scheduled body".to_owned(),
        1_900_000_060_000,
    )
    .expect("reschedule_scheduled_send should build a command")
    {
        CoreCommand::App(AppCommand::RescheduleScheduledSend {
            request_id,
            scheduled_id,
            body,
            send_at_ms,
        }) => {
            assert_eq!(request_id, fake_request_id(35));
            assert_eq!(scheduled_id, "scheduled-1");
            assert_eq!(body, "edited scheduled body");
            assert_eq!(send_at_ms, 1_900_000_060_000);
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_retry_send_command(
        fake_request_id(31),
        active_account_key.clone(),
        room_id.clone(),
        "sdk-txn-1".to_owned(),
    )
    .expect("retry_send should build a command")
    {
        CoreCommand::Timeline(TimelineCommand::RetrySend {
            request_id,
            key,
            transaction_id,
        }) => {
            assert_eq!(request_id, fake_request_id(31));
            assert_eq!(key.account_key, active_account_key);
            assert_eq!(
                key.kind,
                koushi_core::TimelineKind::Room {
                    room_id: room_id.clone()
                }
            );
            assert_eq!(transaction_id, "sdk-txn-1");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_cancel_send_command(
        fake_request_id(32),
        active_account_key.clone(),
        room_id.clone(),
        "sdk-txn-2".to_owned(),
    )
    .expect("cancel_send should build a command")
    {
        CoreCommand::Timeline(TimelineCommand::CancelSend {
            request_id,
            key,
            transaction_id,
        }) => {
            assert_eq!(request_id, fake_request_id(32));
            assert_eq!(key.account_key, active_account_key);
            assert_eq!(
                key.kind,
                koushi_core::TimelineKind::Room {
                    room_id: room_id.clone()
                }
            );
            assert_eq!(transaction_id, "sdk-txn-2");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_forward_message_command(
        fake_request_id(33),
        active_account_key.clone(),
        room_id.clone(),
        "$source-event".to_owned(),
        "!destination:example.invalid".to_owned(),
        "desktop-forward-1".to_owned(),
    )
    .expect("forward_message should build a command")
    {
        CoreCommand::Timeline(TimelineCommand::ForwardMessage {
            request_id,
            key,
            source_event_id,
            destination_room_id,
            transaction_id,
        }) => {
            assert_eq!(request_id, fake_request_id(33));
            assert_eq!(key.account_key, active_account_key);
            assert_eq!(
                key.kind,
                koushi_core::TimelineKind::Room {
                    room_id: room_id.clone()
                }
            );
            assert_eq!(source_event_id, "$source-event");
            assert_eq!(destination_room_id, "!destination:example.invalid");
            assert_eq!(transaction_id, "desktop-forward-1");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_load_message_source_command(
        fake_request_id(34),
        active_account_key.clone(),
        room_id.clone(),
        "$source-event".to_owned(),
    )
    .expect("load_message_source should build a command")
    {
        CoreCommand::Timeline(TimelineCommand::LoadMessageSource {
            request_id,
            key,
            event_id,
        }) => {
            assert_eq!(request_id, fake_request_id(34));
            assert_eq!(key.account_key, active_account_key);
            assert_eq!(
                key.kind,
                koushi_core::TimelineKind::Room {
                    room_id: room_id.clone()
                }
            );
            assert_eq!(event_id, "$source-event");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_request_room_key_command(
        fake_request_id(36),
        active_account_key.clone(),
        room_id.clone(),
        "$source-event".to_owned(),
        koushi_core::KeyRequestOrigin::User,
        Some(TimelineKey {
            account_key: AccountKey("@stale:example.invalid".to_owned()),
            kind: koushi_core::TimelineKind::Thread {
                room_id: room_id.clone(),
                root_event_id: "$thread-root".to_owned(),
            },
        }),
    )
    .expect("request_room_key should build a command")
    {
        CoreCommand::Timeline(TimelineCommand::RequestRoomKey {
            request_id,
            key,
            event_id,
            origin,
        }) => {
            assert_eq!(request_id, fake_request_id(36));
            assert_eq!(key.account_key, active_account_key);
            assert_eq!(
                key.kind,
                koushi_core::TimelineKind::Thread {
                    room_id: room_id.clone(),
                    root_event_id: "$thread-root".to_owned()
                }
            );
            assert_eq!(event_id, "$source-event");
            assert_eq!(origin, koushi_core::KeyRequestOrigin::User);
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_request_late_decryption_command(
        fake_request_id(37),
        active_account_key.clone(),
        room_id.clone(),
        None,
    )
    .expect("request_late_decryption should build a command")
    {
        CoreCommand::Timeline(TimelineCommand::RequestLateDecryption { request_id, key }) => {
            assert_eq!(request_id, fake_request_id(37));
            assert_eq!(key.account_key, active_account_key);
            assert_eq!(
                key.kind,
                koushi_core::TimelineKind::Room {
                    room_id: room_id.clone()
                }
            );
        }
        other => panic!("unexpected command: {other:?}"),
    }

    assert!(
        build_retry_send_command(
            fake_request_id(35),
            active_account_key.clone(),
            room_id.clone(),
            " \t".to_owned()
        )
        .is_none()
    );
    assert!(
        build_cancel_send_command(
            fake_request_id(36),
            active_account_key.clone(),
            room_id.clone(),
            "\n".to_owned()
        )
        .is_none()
    );

    match build_upload_media_command(
        fake_request_id(25),
        active_session_key.clone(),
        active_account_key.clone(),
        room_id.clone(),
        "desktop-media-1".to_owned(),
        "report.pdf".to_owned(),
        "application/pdf".to_owned(),
        vec![1, 2, 3, 4],
        None,
        ImageUploadCompressionMode::Never,
        ImageUploadCompressionPolicy::default(),
        None,
        None,
        None,
    )
    .expect("upload_media should build a command")
    {
        CoreCommand::Timeline(TimelineCommand::UploadAndSendMedia {
            request_id,
            expected_account,
            key,
            transaction_id,
            request,
        }) => {
            assert_eq!(request_id, fake_request_id(25));
            assert_eq!(expected_account, active_session_key);
            assert_eq!(key.account_key, active_account_key);
            assert_eq!(
                key.kind,
                koushi_core::TimelineKind::Room {
                    room_id: room_id.clone()
                }
            );
            assert_eq!(transaction_id, "desktop-media-1");
            assert_eq!(request.filename, "report.pdf");
            assert_eq!(request.mime_type, "application/pdf");
            assert_eq!(request.bytes, vec![1, 2, 3, 4]);
            assert_eq!(request.kind, UploadMediaKind::File);
            assert_eq!(request.caption, None);
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_upload_media_command(
        fake_request_id(26),
        active_session_key.clone(),
        active_account_key.clone(),
        room_id.clone(),
        "desktop-media-2".to_owned(),
        "photo.png".to_owned(),
        "image/png".to_owned(),
        vec![9],
        Some("single **event** caption".to_owned()),
        ImageUploadCompressionMode::Never,
        ImageUploadCompressionPolicy::default(),
        None,
        None,
        None,
    )
    .expect("image upload_media should build a command")
    {
        CoreCommand::Timeline(TimelineCommand::UploadAndSendMedia { request, .. }) => {
            assert_eq!(
                request.kind,
                UploadMediaKind::Image {
                    width: None,
                    height: None
                }
            );
            let caption = request.caption.expect("caption should be preserved");
            assert_eq!(caption.plain_body, "single **event** caption");
            assert_eq!(
                caption.formatted_body.as_deref(),
                Some("single <strong>event</strong> caption")
            );
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_upload_media_command(
        fake_request_id(37),
        active_session_key.clone(),
        active_account_key.clone(),
        room_id.clone(),
        "desktop-media-3".to_owned(),
        "screenshot.jpg".to_owned(),
        "image/jpeg".to_owned(),
        vec![7, 8, 9, 10],
        None,
        ImageUploadCompressionMode::Always,
        ImageUploadCompressionPolicy::default(),
        Some(ImageUploadDimensions {
            width: 1200,
            height: 900,
        }),
        Some(ImageUploadCompressionState {
            mode: koushi_state::ImageUploadCompressionMode::Always,
            policy: ImageUploadCompressionPolicy::default(),
            original: ImageUploadVariantInfo {
                mime_type: "image/jpeg".to_owned(),
                byte_count: 3_200_000,
                dimensions: Some(ImageUploadDimensions {
                    width: 4032,
                    height: 3024,
                }),
            },
            selected: ImageUploadVariantInfo {
                mime_type: "image/jpeg".to_owned(),
                byte_count: 999,
                dimensions: Some(ImageUploadDimensions {
                    width: 1200,
                    height: 900,
                }),
            },
            selected_variant: ImageUploadVariantKind::Compressed,
            skipped_small_image: false,
            metadata_stripped: true,
            thumbnail_refreshed: true,
        }),
        Some(UploadMediaThumbnail {
            mime_type: "image/jpeg".to_owned(),
            bytes: vec![1, 1, 1],
            width: 320,
            height: 240,
        }),
    )
    .expect("compressed image upload_media should build a command")
    {
        CoreCommand::Timeline(TimelineCommand::UploadAndSendMedia { request, .. }) => {
            assert_eq!(
                request.kind,
                UploadMediaKind::Image {
                    width: Some(1200),
                    height: Some(900)
                }
            );
            let compression = request
                .compression
                .expect("image compression contract should be preserved");
            assert_eq!(
                compression.selected_variant,
                ImageUploadVariantKind::Compressed
            );
            assert_eq!(compression.selected.byte_count, 4);
            assert!(compression.metadata_stripped);
            assert!(compression.thumbnail_refreshed);
            assert_eq!(
                request.thumbnail.as_ref().map(|thumbnail| {
                    (
                        thumbnail.mime_type.as_str(),
                        thumbnail.bytes.len(),
                        thumbnail.width,
                        thumbnail.height,
                    )
                }),
                Some(("image/jpeg", 3, 320, 240))
            );
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_download_media_command(
        fake_request_id(27),
        active_account_key.clone(),
        room_id.clone(),
        "$media-event".to_owned(),
    )
    .expect("download_media should build a command")
    {
        CoreCommand::Timeline(TimelineCommand::DownloadMedia {
            request_id,
            key,
            event_id,
            selection,
        }) => {
            assert_eq!(request_id, fake_request_id(27));
            assert_eq!(key.account_key, active_account_key);
            assert_eq!(
                key.kind,
                koushi_core::TimelineKind::Room {
                    room_id: room_id.clone()
                }
            );
            assert_eq!(event_id, "$media-event");
            assert_eq!(selection, MediaDownloadSelection::File);
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_edit_message_command(
        fake_request_id(11),
        active_account_key.clone(),
        room_id.clone(),
        "$event".to_owned(),
        ComposerDocument::from_plain_text(edit_body.clone()),
    )
    .expect("edit_message should build a command")
    {
        CoreCommand::Timeline(TimelineCommand::EditText {
            request_id,
            key,
            event_id,
            document,
        }) => {
            assert_eq!(request_id, fake_request_id(11));
            assert_eq!(key.account_key, active_account_key);
            assert_eq!(
                key.kind,
                koushi_core::TimelineKind::Room {
                    room_id: room_id.clone()
                }
            );
            assert_eq!(event_id, "$event");
            assert_eq!(document.plain_body(), edit_body);
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_redact_message_command(
        fake_request_id(12),
        active_account_key.clone(),
        room_id.clone(),
        "$event".to_owned(),
    ) {
        CoreCommand::Timeline(TimelineCommand::Redact {
            request_id,
            key,
            event_id,
        }) => {
            assert_eq!(request_id, fake_request_id(12));
            assert_eq!(key.account_key, active_account_key);
            assert_eq!(
                key.kind,
                koushi_core::TimelineKind::Room {
                    room_id: room_id.clone()
                }
            );
            assert_eq!(event_id, "$event");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_toggle_reaction_command(
        fake_request_id(13),
        active_account_key.clone(),
        room_id.clone(),
        "$event".to_owned(),
        "👍".to_owned(),
    )
    .expect("toggle_reaction should build a command")
    {
        CoreCommand::Timeline(TimelineCommand::ToggleReaction {
            request_id,
            key,
            event_id,
            reaction_key,
        }) => {
            assert_eq!(request_id, fake_request_id(13));
            assert_eq!(key.account_key, active_account_key);
            assert_eq!(
                key.kind,
                koushi_core::TimelineKind::Room {
                    room_id: room_id.clone()
                }
            );
            assert_eq!(event_id, "$event");
            assert_eq!(reaction_key, "👍");
            let debug = format!(
                "{:?}",
                CoreCommand::Timeline(TimelineCommand::ToggleReaction {
                    request_id: fake_request_id(13),
                    key,
                    event_id,
                    reaction_key,
                })
            );
            assert!(!debug.contains("👍"), "{debug}");
            assert!(!debug.contains("$event"), "{debug}");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_send_reaction_command(
        fake_request_id(25),
        active_account_key.clone(),
        room_id.clone(),
        "$event".to_owned(),
        "👍".to_owned(),
    )
    .expect("send_reaction should build a command")
    {
        CoreCommand::Timeline(TimelineCommand::SendReaction {
            request_id,
            key,
            event_id,
            reaction_key,
        }) => {
            assert_eq!(request_id, fake_request_id(25));
            assert_eq!(key.account_key, active_account_key);
            assert_eq!(
                key.kind,
                koushi_core::TimelineKind::Room {
                    room_id: room_id.clone()
                }
            );
            assert_eq!(event_id, "$event");
            assert_eq!(reaction_key, "👍");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_redact_reaction_command(
        fake_request_id(26),
        active_account_key.clone(),
        room_id.clone(),
        "$event".to_owned(),
        "👍".to_owned(),
        "$reaction".to_owned(),
    )
    .expect("redact_reaction should build a command")
    {
        CoreCommand::Timeline(TimelineCommand::RedactReaction {
            request_id,
            key,
            event_id,
            reaction_key,
            reaction_event_id,
        }) => {
            assert_eq!(request_id, fake_request_id(26));
            assert_eq!(key.account_key, active_account_key);
            assert_eq!(
                key.kind,
                koushi_core::TimelineKind::Room {
                    room_id: room_id.clone()
                }
            );
            assert_eq!(event_id, "$event");
            assert_eq!(reaction_key, "👍");
            assert_eq!(reaction_event_id, "$reaction");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_send_read_receipt_command(
        fake_request_id(28),
        active_account_key.clone(),
        room_id.clone(),
        "$receipt-event".to_owned(),
        None,
    )
    .expect("send_read_receipt should build a command")
    {
        CoreCommand::Timeline(TimelineCommand::SendReadReceipt {
            request_id,
            key,
            event_id,
        }) => {
            assert_eq!(request_id, fake_request_id(28));
            assert_eq!(key.account_key, active_account_key);
            assert_eq!(
                key.kind,
                koushi_core::TimelineKind::Room {
                    room_id: room_id.clone()
                }
            );
            assert_eq!(event_id, "$receipt-event");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_send_read_receipt_command(
        fake_request_id(128),
        active_account_key.clone(),
        room_id.clone(),
        "$thread-receipt-event".to_owned(),
        Some("$thread-root".to_owned()),
    )
    .expect("thread send_read_receipt should build a command")
    {
        CoreCommand::Timeline(TimelineCommand::SendReadReceipt {
            request_id,
            key,
            event_id,
        }) => {
            assert_eq!(request_id, fake_request_id(128));
            assert_eq!(key.account_key, active_account_key);
            assert_eq!(
                key.kind,
                koushi_core::TimelineKind::Thread {
                    room_id: room_id.clone(),
                    root_event_id: "$thread-root".to_owned()
                }
            );
            assert_eq!(event_id, "$thread-receipt-event");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_set_fully_read_command(
        fake_request_id(29),
        active_account_key.clone(),
        room_id.clone(),
        "$fully-read-event".to_owned(),
    )
    .expect("set_fully_read should build a command")
    {
        CoreCommand::Timeline(TimelineCommand::SetFullyRead {
            request_id,
            key,
            event_id,
        }) => {
            assert_eq!(request_id, fake_request_id(29));
            assert_eq!(key.account_key, active_account_key);
            assert_eq!(
                key.kind,
                koushi_core::TimelineKind::Room {
                    room_id: room_id.clone()
                }
            );
            assert_eq!(event_id, "$fully-read-event");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_observe_timeline_viewport_command(
        fake_request_id(31),
        active_account_key.clone(),
        room_id.clone(),
        Some("$first-visible".to_owned()),
        Some("$last-visible".to_owned()),
        Vec::new(),
        false,
        None,
    ) {
        CoreCommand::Timeline(TimelineCommand::ObserveViewport {
            request_id,
            key,
            observation,
        }) => {
            assert_eq!(request_id, fake_request_id(31));
            assert_eq!(key.account_key, active_account_key);
            assert_eq!(
                key.kind,
                koushi_core::TimelineKind::Room {
                    room_id: room_id.clone()
                }
            );
            assert_eq!(
                observation.first_visible_event_id.as_deref(),
                Some("$first-visible")
            );
            assert_eq!(
                observation.last_visible_event_id.as_deref(),
                Some("$last-visible")
            );
            assert!(observation.visible_gap_ids.is_empty());
            assert!(!observation.at_bottom);
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_open_timeline_at_timestamp_command(
        fake_request_id(32),
        room_id.clone(),
        1_718_000_000_000,
    ) {
        CoreCommand::App(AppCommand::OpenTimelineAtTimestamp {
            request_id,
            room_id: command_room_id,
            timestamp_ms,
        }) => {
            assert_eq!(request_id, fake_request_id(32));
            assert_eq!(command_room_id, room_id);
            assert_eq!(timestamp_ms, 1_718_000_000_000);
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_set_typing_command(
        fake_request_id(30),
        active_account_key.clone(),
        room_id.clone(),
        true,
    ) {
        CoreCommand::Timeline(TimelineCommand::SetTyping {
            request_id,
            key,
            is_typing,
        }) => {
            assert_eq!(request_id, fake_request_id(30));
            assert_eq!(key.account_key, active_account_key);
            assert_eq!(
                key.kind,
                koushi_core::TimelineKind::Room {
                    room_id: room_id.clone()
                }
            );
            assert!(is_typing);
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_set_presence_command(fake_request_id(31), PresenceKind::Away) {
        CoreCommand::Account(AccountCommand::SetPresence {
            request_id,
            presence,
        }) => {
            assert_eq!(request_id, fake_request_id(31));
            assert_eq!(presence, PresenceKind::Away);
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_set_display_name_command(fake_request_id(32), Some("Private Display".to_owned())) {
        CoreCommand::Account(AccountCommand::SetDisplayName {
            request_id,
            display_name,
        }) => {
            assert_eq!(request_id, fake_request_id(32));
            assert_eq!(display_name.as_deref(), Some("Private Display"));
            let debug = format!(
                "{:?}",
                CoreCommand::Account(AccountCommand::SetDisplayName {
                    request_id,
                    display_name,
                })
            );
            assert!(!debug.contains("Private Display"), "{debug}");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_set_local_user_alias_command(
        fake_request_id(34),
        "@target:example.invalid".to_owned(),
        Some("Desk Alias".to_owned()),
    ) {
        CoreCommand::Account(AccountCommand::SetLocalUserAlias {
            request_id,
            user_id,
            alias,
        }) => {
            assert_eq!(request_id, fake_request_id(34));
            assert_eq!(user_id, "@target:example.invalid");
            assert_eq!(alias.as_deref(), Some("Desk Alias"));
            let debug = format!(
                "{:?}",
                CoreCommand::Account(AccountCommand::SetLocalUserAlias {
                    request_id,
                    user_id,
                    alias,
                })
            );
            assert!(!debug.contains("@target:example.invalid"), "{debug}");
            assert!(!debug.contains("Desk Alias"), "{debug}");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_set_local_user_alias_command(
        fake_request_id(35),
        "@target:example.invalid".to_owned(),
        None,
    ) {
        CoreCommand::Account(AccountCommand::SetLocalUserAlias {
            request_id,
            user_id,
            alias,
        }) => {
            assert_eq!(request_id, fake_request_id(35));
            assert_eq!(user_id, "@target:example.invalid");
            assert_eq!(alias, None);
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_ignore_user_command(fake_request_id(60), "@ignored:example.invalid".to_owned()) {
        CoreCommand::Account(AccountCommand::IgnoreUser {
            request_id,
            user_id,
        }) => {
            assert_eq!(request_id, fake_request_id(60));
            assert_eq!(user_id, "@ignored:example.invalid");
            let debug = format!(
                "{:?}",
                CoreCommand::Account(AccountCommand::IgnoreUser {
                    request_id,
                    user_id
                })
            );
            assert!(!debug.contains("@ignored:example.invalid"), "{debug}");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_unignore_user_command(fake_request_id(61), "@ignored:example.invalid".to_owned()) {
        CoreCommand::Account(AccountCommand::UnignoreUser {
            request_id,
            user_id,
        }) => {
            assert_eq!(request_id, fake_request_id(61));
            assert_eq!(user_id, "@ignored:example.invalid");
            let debug = format!(
                "{:?}",
                CoreCommand::Account(AccountCommand::UnignoreUser {
                    request_id,
                    user_id
                })
            );
            assert!(!debug.contains("@ignored:example.invalid"), "{debug}");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_report_user_command(
        fake_request_id(62),
        "@reported:example.invalid".to_owned(),
        Some("spam".to_owned()),
    ) {
        CoreCommand::Account(AccountCommand::ReportUser {
            request_id,
            user_id,
            reason,
        }) => {
            assert_eq!(request_id, fake_request_id(62));
            assert_eq!(user_id, "@reported:example.invalid");
            assert_eq!(reason, "spam");
            let debug = format!(
                "{:?}",
                CoreCommand::Account(AccountCommand::ReportUser {
                    request_id,
                    user_id,
                    reason: reason.clone(),
                })
            );
            assert!(!debug.contains("@reported:example.invalid"), "{debug}");
            assert!(!debug.contains("spam"), "{debug}");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_report_content_command(
        fake_request_id(63),
        room_id.clone(),
        "$reported-event".to_owned(),
        Some("abuse".to_owned()),
    ) {
        CoreCommand::Room(RoomCommand::ReportContent {
            request_id,
            room_id: route_room_id,
            event_id,
            reason,
        }) => {
            assert_eq!(request_id, fake_request_id(63));
            assert_eq!(route_room_id, room_id);
            assert_eq!(event_id, "$reported-event");
            assert_eq!(reason.as_deref(), Some("abuse"));
            let debug = format!(
                "{:?}",
                CoreCommand::Room(RoomCommand::ReportContent {
                    request_id,
                    room_id: route_room_id.clone(),
                    event_id: event_id.clone(),
                    reason: reason.clone(),
                })
            );
            assert!(!debug.contains("$reported-event"), "{debug}");
            assert!(!debug.contains("abuse"), "{debug}");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_report_room_command(
        fake_request_id(64),
        room_id.clone(),
        Some("spam room".to_owned()),
    ) {
        CoreCommand::Room(RoomCommand::ReportRoom {
            request_id,
            room_id: route_room_id,
            reason,
        }) => {
            assert_eq!(request_id, fake_request_id(64));
            assert_eq!(route_room_id, room_id);
            assert_eq!(reason, "spam room");
            let debug = format!(
                "{:?}",
                CoreCommand::Room(RoomCommand::ReportRoom {
                    request_id,
                    room_id: route_room_id.clone(),
                    reason: reason.clone(),
                })
            );
            assert!(!debug.contains("spam room"), "{debug}");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_set_avatar_command(
        fake_request_id(33),
        "image/png".to_owned(),
        vec![9, 8, 7, 6],
    ) {
        CoreCommand::Account(AccountCommand::SetAvatar {
            request_id,
            request,
        }) => {
            assert_eq!(request_id, fake_request_id(33));
            assert_eq!(request.mime_type, "image/png");
            assert_eq!(request.bytes, vec![9, 8, 7, 6]);
            let debug = format!(
                "{:?}",
                CoreCommand::Account(AccountCommand::SetAvatar {
                    request_id,
                    request,
                })
            );
            assert!(debug.contains("image/png"), "{debug}");
            assert!(!debug.contains("9, 8, 7, 6"), "{debug}");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_leave_room_command(fake_request_id(13), room_id.clone()) {
        CoreCommand::Room(RoomCommand::LeaveRoom {
            request_id,
            room_id: route_room_id,
        }) => {
            assert_eq!(request_id, fake_request_id(13));
            assert_eq!(route_room_id, room_id);
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_forget_room_command(fake_request_id(14), room_id.clone()) {
        CoreCommand::Room(RoomCommand::ForgetRoom {
            request_id,
            room_id: route_room_id,
        }) => {
            assert_eq!(request_id, fake_request_id(14));
            assert_eq!(route_room_id, room_id);
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_submit_search_command(
        fake_request_id(15),
        query.clone(),
        resolve_search_scope_from_active_room(
            SearchScopeKind::CurrentRoom,
            Some(room_id.clone()),
            Some("!space:example.org".to_owned()),
        ),
    ) {
        CoreCommand::Search(SearchCommand::Query {
            request_id,
            query: route_query,
            scope,
            ..
        }) => {
            assert_eq!(request_id, fake_request_id(15));
            assert_eq!(route_query, query);
            assert_eq!(
                scope,
                SearchScope::CurrentRoom {
                    room_id: room_id.clone()
                }
            );
        }
        other => panic!("unexpected command: {other:?}"),
    }

    assert_eq!(
        resolve_search_scope_from_active_room(SearchScopeKind::CurrentRoom, None, None),
        SearchScope::CurrentRoom {
            room_id: String::new()
        }
    );
    assert_eq!(
        resolve_search_scope_from_active_room(
            SearchScopeKind::CurrentSpace,
            Some(room_id.clone()),
            Some("!space:example.org".to_owned()),
        ),
        SearchScope::CurrentSpace {
            space_id: "!space:example.org".to_owned()
        }
    );
    match build_close_search_command(fake_request_id(16)) {
        CoreCommand::App(AppCommand::CloseSearch { request_id }) => {
            assert_eq!(request_id, fake_request_id(16));
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_create_room_command(
        fake_request_id(17),
        CreateRoomOptions {
            name: "Local QA Room".to_owned(),
            topic: Some("Local topic".to_owned()),
            alias_localpart: Some("local-qa-room".to_owned()),
            encrypted: false,
            visibility: CreateRoomVisibility::Public,
            parent_space: Some(CreateRoomParentSpace {
                space_id: "!space:example.org".to_owned(),
                via_server: "example.org".to_owned(),
            }),
        },
    ) {
        CoreCommand::Room(RoomCommand::CreateRoom {
            request_id,
            options,
        }) => {
            assert_eq!(request_id, fake_request_id(17));
            assert_eq!(options.name, "Local QA Room");
            assert_eq!(options.topic.as_deref(), Some("Local topic"));
            assert_eq!(options.alias_localpart.as_deref(), Some("local-qa-room"));
            assert!(!options.encrypted);
            assert_eq!(options.visibility, CreateRoomVisibility::Public);
            assert_eq!(
                options
                    .parent_space
                    .as_ref()
                    .map(|parent| parent.space_id.as_str()),
                Some("!space:example.org")
            );
            assert_eq!(
                options
                    .parent_space
                    .as_ref()
                    .map(|parent| parent.via_server.as_str()),
                Some("example.org")
            );
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_create_space_command(fake_request_id(17), "Local QA Space".to_owned()) {
        CoreCommand::Room(RoomCommand::CreateSpace { request_id, name }) => {
            assert_eq!(request_id, fake_request_id(17));
            assert_eq!(name, "Local QA Space");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_set_space_child_command(
        fake_request_id(18),
        "!space:example.org".to_owned(),
        "!room:example.org".to_owned(),
        "example.org".to_owned(),
    ) {
        CoreCommand::Room(RoomCommand::SetSpaceChild {
            request_id,
            space_id,
            child_room_id,
            via_server,
        }) => {
            assert_eq!(request_id, fake_request_id(18));
            assert_eq!(space_id, "!space:example.org");
            assert_eq!(child_room_id, "!room:example.org");
            assert_eq!(via_server, "example.org");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_accept_invite_command(fake_request_id(19), "!invite:example.org".to_owned()) {
        CoreCommand::Room(RoomCommand::AcceptInvite {
            request_id,
            room_id,
        }) => {
            assert_eq!(request_id, fake_request_id(19));
            assert_eq!(room_id, "!invite:example.org");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_decline_invite_command(fake_request_id(20), "!decline:example.org".to_owned()) {
        CoreCommand::Room(RoomCommand::DeclineInvite {
            request_id,
            room_id,
        }) => {
            assert_eq!(request_id, fake_request_id(20));
            assert_eq!(room_id, "!decline:example.org");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_start_direct_message_command(fake_request_id(21), "@target:example.org".to_owned())
    {
        CoreCommand::Room(RoomCommand::StartDirectMessage {
            request_id,
            user_id,
        }) => {
            assert_eq!(request_id, fake_request_id(21));
            assert_eq!(user_id, "@target:example.org");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_invite_user_command(
        fake_request_id(22),
        "!room:example.org".to_owned(),
        "@target:example.org".to_owned(),
    ) {
        CoreCommand::Room(RoomCommand::InviteUser {
            request_id,
            room_id,
            user_id,
        }) => {
            assert_eq!(request_id, fake_request_id(22));
            assert_eq!(room_id, "!room:example.org");
            assert_eq!(user_id, "@target:example.org");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_set_room_tag_command(
        fake_request_id(23),
        "!room:example.org".to_owned(),
        RoomTagKind::Favourite,
        Some(0.25),
    ) {
        CoreCommand::Room(RoomCommand::SetTag {
            request_id,
            room_id,
            tag,
            order,
        }) => {
            assert_eq!(request_id, fake_request_id(23));
            assert_eq!(room_id, "!room:example.org");
            assert_eq!(tag, RoomTagKind::Favourite);
            assert_eq!(order, Some(0.25));
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_remove_room_tag_command(
        fake_request_id(24),
        "!room:example.org".to_owned(),
        RoomTagKind::LowPriority,
    ) {
        CoreCommand::Room(RoomCommand::RemoveTag {
            request_id,
            room_id,
            tag,
        }) => {
            assert_eq!(request_id, fake_request_id(24));
            assert_eq!(room_id, "!room:example.org");
            assert_eq!(tag, RoomTagKind::LowPriority);
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_pin_event_command(
        fake_request_id(25),
        "!room:example.org".to_owned(),
        "$event:example.org".to_owned(),
    ) {
        CoreCommand::Room(RoomCommand::PinEvent {
            request_id,
            room_id,
            event_id,
        }) => {
            assert_eq!(request_id, fake_request_id(25));
            assert_eq!(room_id, "!room:example.org");
            assert_eq!(event_id, "$event:example.org");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_unpin_event_command(
        fake_request_id(26),
        "!room:example.org".to_owned(),
        "$event:example.org".to_owned(),
    ) {
        CoreCommand::Room(RoomCommand::UnpinEvent {
            request_id,
            room_id,
            event_id,
        }) => {
            assert_eq!(request_id, fake_request_id(26));
            assert_eq!(room_id, "!room:example.org");
            assert_eq!(event_id, "$event:example.org");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_query_directory_command(
        fake_request_id(27),
        Some("public rooms".to_owned()),
        Some("example.org".to_owned()),
        Some(20),
        Some("page-2".to_owned()),
    ) {
        CoreCommand::Room(RoomCommand::QueryDirectory { request_id, query }) => {
            assert_eq!(request_id, fake_request_id(27));
            assert_eq!(query.term.as_deref(), Some("public rooms"));
            assert_eq!(query.server_name.as_deref(), Some("example.org"));
            assert_eq!(query.limit, Some(20));
            assert_eq!(query.since.as_deref(), Some("page-2"));
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_join_directory_room_command(
        fake_request_id(28),
        "#public:example.org".to_owned(),
        vec!["example.org".to_owned()],
    )
    .expect("directory join should build a command")
    {
        CoreCommand::Room(RoomCommand::JoinDirectoryRoom {
            request_id,
            room_id_or_alias,
            via_servers,
        }) => {
            assert_eq!(request_id, fake_request_id(28));
            assert_eq!(room_id_or_alias, "#public:example.org");
            assert_eq!(via_servers, vec!["example.org".to_owned()]);
        }
        other => panic!("unexpected command: {other:?}"),
    }

    assert!(
        build_join_directory_room_command(fake_request_id(29), "   ".to_owned(), Vec::new(),)
            .is_none()
    );

    match build_join_room_command(fake_request_id(290), " !child:example.org ".to_owned())
        .expect("room join should build a command")
    {
        CoreCommand::Room(RoomCommand::JoinRoom {
            request_id,
            room_id,
        }) => {
            assert_eq!(request_id, fake_request_id(290));
            assert_eq!(room_id, "!child:example.org");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    assert!(build_join_room_command(fake_request_id(291), "   ".to_owned()).is_none());

    match build_load_room_settings_command(fake_request_id(30), "!room:example.org".to_owned()) {
        CoreCommand::Room(RoomCommand::LoadRoomSettings {
            request_id,
            room_id,
        }) => {
            assert_eq!(request_id, fake_request_id(30));
            assert_eq!(room_id, "!room:example.org");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    for (offset, change) in [
        (
            31,
            RoomSettingChange::Name(Some("Private room name".to_owned())),
        ),
        (
            32,
            RoomSettingChange::Topic(Some("Private room topic".to_owned())),
        ),
        (
            33,
            RoomSettingChange::AvatarUrl(Some("mxc://example.org/private".to_owned())),
        ),
        (34, RoomSettingChange::JoinRule(RoomJoinRule::Invite)),
        (
            35,
            RoomSettingChange::HistoryVisibility(RoomHistoryVisibility::Shared),
        ),
    ] {
        match build_update_room_setting_command(
            fake_request_id(offset),
            "!room:example.org".to_owned(),
            change.clone(),
        ) {
            CoreCommand::Room(RoomCommand::UpdateRoomSetting {
                request_id,
                room_id,
                change: routed_change,
            }) => {
                assert_eq!(request_id, fake_request_id(offset));
                assert_eq!(room_id, "!room:example.org");
                assert_eq!(routed_change, change);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    match build_moderate_room_member_command(
        fake_request_id(36),
        "!room:example.org".to_owned(),
        "@target:example.org".to_owned(),
        RoomModerationAction::Kick,
        Some("private reason".to_owned()),
    ) {
        CoreCommand::Room(RoomCommand::ModerateRoomMember {
            request_id,
            room_id,
            target_user_id,
            action,
            reason,
        }) => {
            assert_eq!(request_id, fake_request_id(36));
            assert_eq!(room_id, "!room:example.org");
            assert_eq!(target_user_id, "@target:example.org");
            assert_eq!(action, RoomModerationAction::Kick);
            assert_eq!(reason.as_deref(), Some("private reason"));
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_send_reply_command(
        fake_request_id(23),
        active_account_key.clone(),
        room_id.clone(),
        "desktop-reply-1".to_owned(),
        "$root".to_owned(),
        ComposerDocument::from_plain_text("reply body"),
    )
    .expect("send_reply should build a command")
    {
        CoreCommand::Timeline(TimelineCommand::SendReply {
            request_id,
            key,
            transaction_id,
            in_reply_to_event_id,
            document,
        }) => {
            assert_eq!(request_id, fake_request_id(23));
            assert_eq!(key.account_key, active_account_key);
            assert_eq!(
                key.kind,
                koushi_core::TimelineKind::Room {
                    room_id: room_id.clone()
                }
            );
            assert_eq!(transaction_id, "desktop-reply-1");
            assert_eq!(in_reply_to_event_id, "$root");
            assert_eq!(document.plain_body(), "reply body");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_send_thread_reply_command(
        fake_request_id(24),
        active_account_key.clone(),
        room_id.clone(),
        "$root".to_owned(),
        "desktop-thread-reply-1".to_owned(),
        ComposerDocument::from_plain_text("thread reply body"),
    )
    .expect("send_thread_reply should build a command")
    {
        CoreCommand::Timeline(TimelineCommand::SendReply {
            request_id,
            key,
            transaction_id,
            in_reply_to_event_id,
            document,
        }) => {
            assert_eq!(request_id, fake_request_id(24));
            assert_eq!(key.account_key, active_account_key);
            assert_eq!(
                key.kind,
                koushi_core::TimelineKind::Thread {
                    room_id: room_id.clone(),
                    root_event_id: "$root".to_owned(),
                }
            );
            assert_eq!(transaction_id, "desktop-thread-reply-1");
            assert_eq!(in_reply_to_event_id, "$root");
            assert_eq!(document.plain_body(), "thread reply body");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_set_thread_composer_draft_command(
        fake_request_id(21),
        active_session_key.clone(),
        room_id.clone(),
        "$root".to_owned(),
        "thread draft".into(),
        9.into(),
    ) {
        CoreCommand::App(AppCommand::SetThreadComposerDraft {
            request_id,
            expected_account,
            room_id: command_room_id,
            root_event_id,
            document,
            revision,
        }) => {
            assert_eq!(request_id, fake_request_id(21));
            assert_eq!(expected_account, active_session_key);
            assert_eq!(command_room_id, room_id);
            assert_eq!(root_event_id, "$root");
            assert_eq!(document.plain_body(), "thread draft");
            assert_eq!(revision, 9.into());
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_set_composer_draft_command(
        fake_request_id(22),
        active_session_key.clone(),
        room_id.clone(),
        "room draft".into(),
        10.into(),
    ) {
        CoreCommand::App(AppCommand::SetComposerDraft {
            request_id,
            expected_account,
            room_id: command_room_id,
            document,
            revision,
        }) => {
            assert_eq!(request_id, fake_request_id(22));
            assert_eq!(expected_account, active_session_key);
            assert_eq!(command_room_id, room_id);
            assert_eq!(document.plain_body(), "room draft");
            assert_eq!(revision, 10.into());
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_open_activity_command(fake_request_id(37)) {
        CoreCommand::App(AppCommand::OpenActivity { request_id }) => {
            assert_eq!(request_id, fake_request_id(37));
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_set_activity_tab_command(fake_request_id(38), ActivityTab::Unread) {
        CoreCommand::App(AppCommand::SetActivityTab { request_id, tab }) => {
            assert_eq!(request_id, fake_request_id(38));
            assert_eq!(tab, ActivityTab::Unread);
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_paginate_activity_command(
        fake_request_id(39),
        ActivityTab::Recent,
        Some("page-2".to_owned()),
    ) {
        CoreCommand::App(AppCommand::PaginateActivity {
            request_id,
            tab,
            cursor,
        }) => {
            assert_eq!(request_id, fake_request_id(39));
            assert_eq!(tab, ActivityTab::Recent);
            assert_eq!(cursor.as_deref(), Some("page-2"));
        }
        other => panic!("unexpected command: {other:?}"),
    }

    assert!(matches!(
        build_paginate_activity_command(
            fake_request_id(40),
            ActivityTab::Unread,
            Some("  ".to_owned())
        ),
        CoreCommand::App(AppCommand::PaginateActivity { cursor: None, .. })
    ));

    let target = ActivityMarkReadTarget::Room {
        room_id: "!room:example.org".to_owned(),
        up_to_event_id: "$event:example.org".to_owned(),
    };
    match build_mark_activity_read_command(fake_request_id(41), target.clone()) {
        CoreCommand::App(AppCommand::MarkActivityRead {
            request_id,
            target: routed_target,
        }) => {
            assert_eq!(request_id, fake_request_id(41));
            assert_eq!(routed_target, target);
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_close_activity_command(fake_request_id(42)) {
        CoreCommand::App(AppCommand::CloseActivity { request_id }) => {
            assert_eq!(request_id, fake_request_id(42));
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_retry_activity_resolution_command(fake_request_id(43)) {
        CoreCommand::App(AppCommand::RetryActivityResolution { request_id }) => {
            assert_eq!(request_id, fake_request_id(43));
        }
        other => panic!("unexpected command: {other:?}"),
    }

    let files_scope = koushi_state::FilesViewScope::Room {
        room_id: "!room:example.org".to_owned(),
    };
    let files_filter = koushi_state::AttachmentFilter {
        kinds: vec![koushi_state::AttachmentKind::Image],
        filename_query: Some("cat".to_owned()),
    };
    match build_open_files_view_command(
        fake_request_id(65),
        files_scope.clone(),
        files_filter.clone(),
        koushi_state::AttachmentSort::Filename,
    ) {
        CoreCommand::App(AppCommand::OpenFilesView {
            request_id,
            scope,
            filter,
            sort,
        }) => {
            assert_eq!(request_id, fake_request_id(65));
            assert_eq!(scope, files_scope);
            assert_eq!(filter, files_filter);
            assert!(matches!(sort, koushi_state::AttachmentSort::Filename));
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_close_files_view_command(fake_request_id(66)) {
        CoreCommand::App(AppCommand::CloseFilesView { request_id }) => {
            assert_eq!(request_id, fake_request_id(66));
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_open_thread_command(
        fake_request_id(67),
        "!room:example.org".to_owned(),
        "$root:example.org".to_owned(),
        ThreadOpenIntent::NewThreadDraft,
    ) {
        CoreCommand::App(AppCommand::OpenThread {
            request_id, intent, ..
        }) => {
            assert_eq!(request_id, fake_request_id(67));
            assert_eq!(intent, ThreadOpenIntent::NewThreadDraft);
        }
        other => panic!("unexpected command: {other:?}"),
    }

    match build_update_room_member_role_command(
        fake_request_id(37),
        "!room:example.org".to_owned(),
        "@target:example.org".to_owned(),
        50,
    ) {
        CoreCommand::Room(RoomCommand::UpdateRoomMemberRole {
            request_id,
            room_id,
            target_user_id,
            power_level,
        }) => {
            assert_eq!(request_id, fake_request_id(37));
            assert_eq!(room_id, "!room:example.org");
            assert_eq!(target_user_id, "@target:example.org");
            assert_eq!(power_level, 50);
        }
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn every_tauri_command_is_registered_in_generate_handler() {
    let commands_source = production_source();
    let lib_source = include_str!("../lib.rs");
    let handler_start = lib_source
        .find("tauri::generate_handler![")
        .expect("generate_handler must exist in lib.rs");
    let handler_end = lib_source[handler_start..]
        .find(']')
        .map(|pos| handler_start + pos)
        .expect("generate_handler must close");
    let handler_block = &lib_source[handler_start..handler_end];

    let marker = "#[tauri::command]";
    let mut found = 0usize;
    for (idx, _) in commands_source.match_indices(marker) {
        let after = commands_source[idx + marker.len()..].trim_start();
        let Some(fn_rest) = after.strip_prefix("pub async fn ") else {
            continue;
        };
        let name: String = fn_rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if name.is_empty() {
            continue;
        }
        found += 1;
        assert!(
            handler_block
                .lines()
                .any(|line| line.contains("commands::") && line.contains(&format!("::{name}"))),
            "Tauri command {name} is defined with #[tauri::command] but is not registered                  in generate_handler! (lib.rs); add commands::<module>::{name} to the invoke handler"
        );
    }
    assert!(
        found > 0,
        "no #[tauri::command] functions found in the command sources"
    );
}

#[test]
fn tauri_command_routes_redact_secret_bearing_values_from_debug() {
    let account_key = AccountKey("@alice:example.org".to_owned());
    let room_id = "!room:example.org".to_owned();
    let login = build_submit_login_command(
        fake_request_id(16),
        LoginRequest {
            homeserver: "https://matrix.example.org".to_owned(),
            username: "alice".to_owned(),
            password: AuthSecret::new("password-123"),
            device_display_name: Some("Laptop".to_owned()),
        },
        DisplayPlatform::Linux,
    );
    let recovery =
        build_submit_recovery_command(fake_request_id(17), AuthSecret::new("recovery-123"));
    let send = build_send_text_command(
        fake_request_id(18),
        account_key.clone(),
        room_id.clone(),
        "desktop-18".to_owned(),
        ComposerDocument::from_plain_text("sensitive body"),
    )
    .expect("send_text should build a command");
    let edit = build_edit_message_command(
        fake_request_id(19),
        account_key,
        room_id,
        "$event".to_owned(),
        ComposerDocument::from_plain_text("sensitive edit body"),
    )
    .expect("edit_message should build a command");
    let upload = build_upload_media_command(
        fake_request_id(21),
        synthetic_session_key(),
        AccountKey("@alice:example.org".to_owned()),
        "!room:example.org".to_owned(),
        "desktop-media-sensitive".to_owned(),
        "secret-filename.pdf".to_owned(),
        "application/pdf".to_owned(),
        b"secret media bytes".to_vec(),
        Some("secret media caption".to_owned()),
        ImageUploadCompressionMode::Never,
        ImageUploadCompressionPolicy::default(),
        None,
        None,
        None,
    )
    .expect("upload_media should build a command");
    let download = build_download_media_command(
        fake_request_id(22),
        AccountKey("@alice:example.org".to_owned()),
        "!room:example.org".to_owned(),
        "$secret-media-event".to_owned(),
    )
    .expect("download_media should build a command");
    let search = build_submit_search_command(
        fake_request_id(20),
        "secret search terms".to_owned(),
        resolve_search_scope_from_active_room(
            SearchScopeKind::CurrentRoom,
            Some("!room:example.org".to_owned()),
            None,
        ),
    );
    let room_key_export = build_export_room_keys_command(
        fake_request_id(23),
        "/tmp/private-room-key-export.txt".to_owned(),
        AuthSecret::new("room-key-transfer-phrase"),
    );
    let room_key_import = build_import_room_keys_command(
        fake_request_id(24),
        "/tmp/private-room-key-import.txt".to_owned(),
        AuthSecret::new("room-key-transfer-phrase"),
    );
    let secure_backup_setup = build_bootstrap_secure_backup_command(
        fake_request_id(25),
        Some(AuthSecret::new("backup-setup-phrase")),
        Some("/tmp/private-recovery-artifact.txt".to_owned()),
        false,
    );
    let secure_backup_change = build_change_secure_backup_passphrase_command(
        fake_request_id(26),
        AuthSecret::new("old-backup-phrase"),
        AuthSecret::new("new-backup-phrase"),
        Some("/tmp/private-recovery-artifact.txt".to_owned()),
    );

    for (command, secret) in [
        (&login, "password-123"),
        (&recovery, "recovery-123"),
        (&send, "sensitive body"),
        (&edit, "sensitive edit body"),
        (&upload, "secret-filename.pdf"),
        (&upload, "secret media bytes"),
        (&download, "$secret-media-event"),
        (&search, "secret search terms"),
        (&room_key_export, "/tmp/private-room-key-export.txt"),
        (&room_key_export, "room-key-transfer-phrase"),
        (&room_key_import, "/tmp/private-room-key-import.txt"),
        (&room_key_import, "room-key-transfer-phrase"),
        (&secure_backup_setup, "backup-setup-phrase"),
        (&secure_backup_setup, "/tmp/private-recovery-artifact.txt"),
        (&secure_backup_change, "old-backup-phrase"),
        (&secure_backup_change, "new-backup-phrase"),
        (&secure_backup_change, "/tmp/private-recovery-artifact.txt"),
    ] {
        let debug = format!("{command:?}");
        assert!(
            !debug.contains(secret),
            "Debug output leaked a secret: {debug}"
        );
    }
}

#[test]
fn tauri_diagnostics_record_without_stderr_environment_switch() {
    let request_id = RequestId {
        connection_id: koushi_core::RuntimeConnectionId(99),
        sequence: 7,
    };

    trace_tauri_timeline_command("submit", "diagnostic_test", request_id);
    trace_tauri_timeline_command_elapsed("done", "diagnostic_test", request_id, 3);

    let records = koushi_diagnostics::snapshot().records;
    assert!(records.iter().any(|record| {
        record.event.source == "desktop.timeline"
            && record.event.stage == "submit"
            && koushi_diagnostics::format_event(&record.event).contains("operation=diagnostic_test")
    }));
    assert!(records.iter().any(|record| {
        record.event.source == "desktop.timeline" && record.event.stage == "done"
    }));
}

#[test]
fn env_unset_real_search_and_select_producers_are_private_data_free() {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "commands::contracts::env_unset_real_search_and_select_producers_child",
            "--ignored",
            "--nocapture",
        ])
        .env_remove("KOUSHI_SUBSCRIBE_TRACE")
        .env_remove("KOUSHI_SEARCH_TRACE")
        .output()
        .expect("env-unset diagnostic child should run");
    assert!(output.status.success(), "child failed: {output:?}");
    let stdout = String::from_utf8(output.stdout).expect("child stdout should be utf8");
    let snapshot: serde_json::Value = serde_json::from_str(
        stdout
            .lines()
            .find(|line| line.starts_with('{'))
            .expect("child should print one JSON snapshot"),
    )
    .expect("child output should be a JSON snapshot");
    let records = snapshot["records"]
        .as_array()
        .expect("records should be an array");
    let fields = |source: &str, stage: &str| {
        records
            .iter()
            .find(|record| record["event"]["source"] == source && record["event"]["stage"] == stage)
            .and_then(|record| record["event"]["fields"].as_array())
            .expect("expected diagnostic record")
    };
    let field = |fields: &[serde_json::Value], key: &str| {
        fields
            .iter()
            .find(|field| field["key"] == key)
            .map(|field| field["value"].clone())
            .expect("expected typed field")
    };
    let search_fields = fields("desktop.search", "submit");
    assert_eq!(
        field(search_fields, "ui_scope"),
        serde_json::json!({"kind":"token","value":"current_room"})
    );
    assert_eq!(
        field(search_fields, "resolved_scope"),
        serde_json::json!({"kind":"token","value":"current_room"})
    );
    assert_eq!(
        field(search_fields, "query_bytes"),
        serde_json::json!({"kind":"count","value":161})
    );
    assert_eq!(
        field(search_fields, "query_chars"),
        serde_json::json!({"kind":"count","value":161})
    );
    assert_eq!(field(search_fields, "request_id")["kind"], "request_id");

    let select_fields = records
        .iter()
        .filter(|record| {
            record["event"]["source"] == "desktop.select" && record["event"]["stage"] == "ok_intent"
        })
        .map(|record| record["event"]["fields"].as_array().expect("select fields"))
        .collect::<Vec<_>>();
    assert_eq!(select_fields.len(), 2);
    for fields in &select_fields {
        assert_eq!(
            field(fields, "events"),
            serde_json::json!({"kind":"count","value":1})
        );
        assert_eq!(
            field(fields, "state_changed"),
            serde_json::json!({"kind":"count","value":0})
        );
        assert_eq!(
            field(fields, "state_delta"),
            serde_json::json!({"kind":"count","value":0})
        );
        assert_eq!(
            field(fields, "active"),
            serde_json::json!({"kind":"token","value":"selected"})
        );
    }
    let outcome_active = select_fields
        .iter()
        .map(|fields| {
            (
                field(fields, "outcome").clone(),
                field(fields, "active").clone(),
            )
        })
        .collect::<Vec<_>>();
    assert!(outcome_active.contains(&(
        serde_json::json!({"kind":"token","value":"committed"}),
        serde_json::json!({"kind":"token","value":"selected"}),
    )));
    assert!(outcome_active.contains(&(
        serde_json::json!({"kind":"token","value":"already_active"}),
        serde_json::json!({"kind":"token","value":"selected"}),
    )));
    let serialized_snapshot =
        serde_json::to_string(&snapshot).expect("parsed diagnostic snapshot should serialize");
    for private_value in [
        "synthetic-room-id",
        "synthetic-query-text",
        "synthetic-event-id",
        "synthetic-user-id",
        "synthetic-body-text",
        "https://synthetic.example/path",
        "/synthetic/private/path",
    ] {
        assert!(
            !serialized_snapshot.contains(private_value),
            "diagnostic snapshot leaked {private_value}"
        );
    }
}

#[test]
#[ignore]
fn env_unset_real_search_and_select_producers_child() {
    assert!(std::env::var_os("KOUSHI_SUBSCRIBE_TRACE").is_none());
    assert!(std::env::var_os("KOUSHI_SEARCH_TRACE").is_none());

    let async_runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("test runtime should build");
    async_runtime.block_on(async {
        let data_dir = tempfile::tempdir().expect("runtime data dir should be created");
        let runtime = koushi_core::CoreRuntime::start_with_data_dir(data_dir.path().to_owned());
        let connection = runtime.attach();
        let state = super::CoreRuntimeState {
            runtime,
            connection: tokio::sync::Mutex::new(connection),
            composer_draft_transport: std::sync::Mutex::new(
                crate::ComposerDraftTransportIdentities::default(),
            ),
            timeline_items_count: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            _forwarder_task: None,
            native_window_focus_generation: std::sync::atomic::AtomicU64::new(0),
            viewport_sync_generation: crate::viewport_sync::ViewportSyncGeneration::default(),
        };
        super::search::submit_search_production_path(
            SYNTHETIC_QUERY.to_owned(),
            SearchScopeKind::CurrentRoom,
            SearchScope::CurrentRoom {
                room_id: "synthetic-room-id".to_owned(),
            },
            &state,
            &ScriptedSearchPathIo,
        )
        .await
        .expect("production search path should reach searching state");

        let request_id = RequestId {
            connection_id: koushi_core::RuntimeConnectionId(101),
            sequence: 9,
        };
        for outcome in [
            IntentOutcome::Committed,
            IntentOutcome::BenignNoOp(IntentNoOpReason::AlreadyActive),
        ] {
            let mut source = ScriptedSelectSource {
                snapshot: AppState::default(),
                events: VecDeque::from([Ok(CoreEvent::IntentLifecycle {
                    request_id,
                    outcome,
                })]),
            };
            super::navigation::wait_for_selected_room(
                &mut source,
                request_id,
                "synthetic-room-id",
                std::time::Duration::from_millis(10),
            )
            .await
            .expect("scripted intent event should select the room");
        }
    });

    let serialized = serde_json::to_string(&koushi_diagnostics::snapshot())
        .expect("diagnostic snapshot should serialize");
    println!("{serialized}");
}
