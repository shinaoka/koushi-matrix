use super::{
    CloseRequestedAction, MacosCloseRequestedAction, QuitRequestAction, QuitStage,
    claim_core_shutdown, close_requested_action, desktop_menu_items, desktop_standard_menu_items,
    macos_close_requested_action, next_native_window_focus_generation,
    observed_native_window_focus, qa_control_pipe_path_from_env_value,
    qa_login_pipe_path_from_env_value, quit_request_action, restore_session_enabled_from_env_value,
    saved_sessions_disabled_from_env_value, window_event_should_stop_background_tasks,
};
use crate::commands::diagnostics::parse_qa_login_pipe_payload;
use std::path::Path;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

#[path = "thumbnail_protocol_test_support.rs"]
mod thumbnail_protocol_test_support;

#[test]
fn main_window_overlay_permission_contract() {
    let capability: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/capabilities/windows-overlay.json"
    )))
    .expect("Windows overlay capability must be valid JSON");
    assert_eq!(capability["identifier"], "windows-overlay");
    assert_eq!(capability["platforms"], serde_json::json!(["windows"]));
    assert_eq!(capability["windows"], serde_json::json!(["main"]));
    let permissions = capability["permissions"]
        .as_array()
        .expect("main capability permissions must be an array");
    assert!(
        permissions
            .iter()
            .any(|permission| permission == "core:window:allow-set-overlay-icon"),
        "main Windows window must explicitly admit the overlay command"
    );
}

#[test]
fn restore_session_env_value_can_start_tauri_signed_out() {
    assert!(!restore_session_enabled_from_env_value(Some("0")));
    assert!(!restore_session_enabled_from_env_value(Some("false")));
    assert!(!restore_session_enabled_from_env_value(Some("signed-out")));
    assert!(restore_session_enabled_from_env_value(None));
    assert!(restore_session_enabled_from_env_value(Some("1")));
}

#[test]
fn saved_sessions_env_value_can_disable_keychain_reads_for_gui_smoke() {
    assert!(saved_sessions_disabled_from_env_value(Some("1")));
    assert!(saved_sessions_disabled_from_env_value(Some("true")));
    assert!(saved_sessions_disabled_from_env_value(Some("yes")));
    assert!(!saved_sessions_disabled_from_env_value(None));
    assert!(!saved_sessions_disabled_from_env_value(Some("0")));
}

#[test]
fn keychain_persistence_env_value_can_disable_os_keychain_for_gui_smoke() {
    assert!(super::keychain_persistence_disabled_from_env_value(Some(
        "1"
    )));
    assert!(super::keychain_persistence_disabled_from_env_value(Some(
        "true"
    )));
    assert!(super::keychain_persistence_disabled_from_env_value(Some(
        "yes"
    )));
    assert!(!super::keychain_persistence_disabled_from_env_value(None));
    assert!(!super::keychain_persistence_disabled_from_env_value(Some(
        "0"
    )));
}

#[test]
fn renderable_thumbnail_protocol_serves_known_cached_bytes() {
    if !thumbnail_protocol_test_support::is_child() {
        thumbnail_protocol_test_support::run_isolated();
        return;
    }
    let ready = koushi_core::renderable_thumbnail::store_renderable_thumbnail(
        koushi_core::renderable_thumbnail::RenderableThumbnailKind::Avatar,
        "mxc://example.test/avatar",
        b"protocol-bytes".to_vec(),
    )
    .expect("protocol fixture is within the thumbnail cache bound");
    let source_ref = match ready {
        koushi_state::AvatarThumbnailState::Ready { source_ref, .. } => source_ref,
        other => panic!("unexpected thumbnail state: {other:?}"),
    };
    thumbnail_protocol_test_support::after_thumbnail_stored();
    let response = super::renderable_thumbnail_protocol_response(
        tauri::http::Request::builder()
            .uri(format!("koushi-thumbnail://localhost/{source_ref}"))
            .body(Vec::new())
            .expect("request"),
    );
    assert_eq!(response.status(), tauri::http::StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(tauri::http::header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("no-store")
    );
    assert_eq!(
        response
            .headers()
            .get(tauri::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/octet-stream")
    );
    assert_eq!(
        response
            .headers()
            .get("X-Content-Type-Options")
            .and_then(|value| value.to_str().ok()),
        Some("nosniff")
    );
    assert_eq!(response.body(), &b"protocol-bytes".to_vec());
}

#[test]
fn renderable_thumbnail_protocol_rejects_unknown_refs() {
    let response = super::renderable_thumbnail_protocol_response(
        tauri::http::Request::builder()
            .uri("koushi-thumbnail://localhost/avatar/unknown")
            .body(Vec::new())
            .expect("request"),
    );
    assert_eq!(response.status(), tauri::http::StatusCode::NOT_FOUND);
    assert_eq!(
        response
            .headers()
            .get("X-Content-Type-Options")
            .and_then(|value| value.to_str().ok()),
        Some("nosniff")
    );
}

#[test]
fn qa_login_pipe_env_uses_path_only() {
    assert_eq!(
        qa_login_pipe_path_from_env_value(Some(" /tmp/koushi-desktop-login.pipe ")),
        Some(Path::new("/tmp/koushi-desktop-login.pipe").to_path_buf())
    );
    assert_eq!(qa_login_pipe_path_from_env_value(Some("   ")), None);
    assert_eq!(qa_login_pipe_path_from_env_value(None), None);
}

#[test]
fn qa_control_pipe_env_uses_path_only() {
    assert_eq!(
        qa_control_pipe_path_from_env_value(Some(" /tmp/koushi-desktop-control.pipe ")),
        Some(Path::new("/tmp/koushi-desktop-control.pipe").to_path_buf())
    );
    assert_eq!(qa_control_pipe_path_from_env_value(Some("   ")), None);
    assert_eq!(qa_control_pipe_path_from_env_value(None), None);
}

/// Release builds must NEVER read the QA control pipe env var. The pipe is a
/// debug/test-only logout-cleanup surface, so its const, helpers, and reader
/// spawn must all sit behind the same `#[cfg(any(debug_assertions, test))]`
/// compile-time gate as the QA login pipe (engineering-rules: Secrets rule
/// 2). This source-level assertion is the release gate: a release binary
/// cannot compile the env read at all.

#[test]
fn qa_login_pipe_payload_maps_to_login_request_without_debugging_secret() {
    let request = parse_qa_login_pipe_payload(
            r#"{"homeserver":"https://matrix.example.org","username":"fixture-user","password":"synthetic-password","device_display_name":"Koushi GUI Smoke","recovery_secret":"synthetic-recovery-secret"}"#,
    )
    .expect("payload should parse");

    assert_eq!(request.login.homeserver, "https://matrix.example.org");
    assert_eq!(request.login.username, "fixture-user");
    assert_eq!(request.login.password.expose_secret(), "synthetic-password");
    assert_eq!(
        request.login.device_display_name.as_deref(),
        Some("Koushi GUI Smoke")
    );
    assert_eq!(
        request
            .recovery_secret
            .as_ref()
            .map(|secret| secret.expose_secret()),
        Some("synthetic-recovery-secret")
    );
    assert!(!format!("{request:?}").contains("synthetic-password"));
    assert!(!format!("{request:?}").contains("synthetic-recovery-secret"));
}

#[test]
fn observed_native_window_focus_extracts_only_focus_events() {
    assert_eq!(
        observed_native_window_focus(&tauri::WindowEvent::Focused(true)),
        Some(true)
    );
    assert_eq!(
        observed_native_window_focus(&tauri::WindowEvent::Focused(false)),
        Some(false)
    );
    assert_eq!(
        observed_native_window_focus(&tauri::WindowEvent::Resized(tauri::PhysicalSize::new(
            1280, 820
        ))),
        None
    );
    assert_eq!(
        observed_native_window_focus(&tauri::WindowEvent::Moved(tauri::PhysicalPosition::new(
            30, 50
        ))),
        None
    );
    assert_eq!(
        observed_native_window_focus(&tauri::WindowEvent::Destroyed),
        None
    );
}

#[test]
fn native_window_focus_generation_is_monotonic_and_exhaustion_safe() {
    let counter = AtomicU64::new(0);
    assert_eq!(next_native_window_focus_generation(&counter), Some(1));
    assert_eq!(next_native_window_focus_generation(&counter), Some(2));

    let exhausted = AtomicU64::new(u64::MAX);
    assert_eq!(next_native_window_focus_generation(&exhausted), None);
    assert_eq!(exhausted.load(Ordering::Relaxed), u64::MAX);
}

#[test]
fn window_event_should_stop_background_tasks_on_shutdown() {
    assert!(window_event_should_stop_background_tasks(
        &tauri::WindowEvent::Destroyed
    ));
    assert!(!window_event_should_stop_background_tasks(
        &tauri::WindowEvent::Focused(false)
    ));
    assert!(!window_event_should_stop_background_tasks(
        &tauri::WindowEvent::Resized(tauri::PhysicalSize::new(1280, 820))
    ));
}

#[test]
fn macos_close_requested_exits_fullscreen_before_hiding() {
    assert_eq!(
        macos_close_requested_action(Some(true)),
        MacosCloseRequestedAction::ExitFullscreenAndHide
    );
    assert_eq!(
        macos_close_requested_action(Some(false)),
        MacosCloseRequestedAction::Hide
    );
    assert_eq!(
        macos_close_requested_action(None),
        MacosCloseRequestedAction::Hide
    );
}

#[test]
fn oidc_callback_url_accepts_only_expected_auth_callback_shape() {
    assert!(super::is_oidc_callback_url(
        "com.github.shinaoka.koushi-matrix:/auth/callback"
    ));
    assert!(super::is_oidc_callback_url(
        "com.github.shinaoka.koushi-matrix:/auth/callback?code=synthetic&state=synthetic"
    ));
    // Slash-count tolerance: URL normalization along the browser → OS
    // opener → deep-link path may add authority slashes.
    assert!(super::is_oidc_callback_url(
        "com.github.shinaoka.koushi-matrix://auth/callback?code=synthetic"
    ));
    assert!(!super::is_oidc_callback_url(
        "com.github.shinaoka.koushi-matrix:/event"
    ));
    assert!(!super::is_oidc_callback_url(
        "com.github.shinaoka.koushi-matrix:/auth/callback-extra?code=synthetic"
    ));
    assert!(!super::is_oidc_callback_url(
        "koushi-desktop://auth/callback?code=synthetic"
    ));
    assert!(!super::is_oidc_callback_url(
        "https://auth.example.test/callback?code=synthetic"
    ));
}

#[test]
fn oidc_callback_state_requires_one_nonempty_state_value() {
    assert_eq!(
        super::oidc_callback_state(
            "com.github.shinaoka.koushi-matrix:/auth/callback?code=synthetic&state=attempt%2Fone"
        ),
        Some("attempt/one".to_owned())
    );
    assert_eq!(
        super::oidc_callback_state(
            "com.github.shinaoka.koushi-matrix:/auth/callback?code=synthetic"
        ),
        None
    );
    assert_eq!(
        super::oidc_callback_state("com.github.shinaoka.koushi-matrix:/auth/callback?state="),
        None
    );
    assert_eq!(
        super::oidc_callback_state(
            "com.github.shinaoka.koushi-matrix:/auth/callback?state=one&state=two"
        ),
        None
    );
}

#[cfg(target_os = "linux")]
#[test]
fn linux_deep_link_desktop_entry_uses_xdg_open_compatible_exec() {
    let generated = "[Desktop Entry]\nExec=\"/opt/koushi/koushi-desktop\" %u\n";
    let repaired = super::repair_linux_deep_link_desktop_entry_contents(generated);
    assert_eq!(
        repaired,
        "[Desktop Entry]\nExec=/opt/koushi/koushi-desktop %u\n"
    );

    let path_with_spaces = "[Desktop Entry]\nExec=\"/opt/Koushi Desktop/koushi-desktop\" %u\n";
    assert_eq!(
        super::repair_linux_deep_link_desktop_entry_contents(path_with_spaces),
        path_with_spaces
    );
}

#[test]
fn desktop_menu_items_include_element_compatible_shortcuts() {
    let items = desktop_menu_items();

    assert!(items.iter().any(|item| {
        item.id == "open_user_settings" && item.accelerator == "CmdOrCtrl+," && item.menu == "app"
    }));
    assert!(
        items
            .iter()
            .any(|item| item.id == "sign_out" && item.accelerator.is_empty() && item.menu == "app")
    );
    let about_index = items
        .iter()
        .position(|item| item.id == "about_koushi")
        .expect("native About Koushi menu item should exist");
    assert_eq!(about_index, 0);
    assert_eq!(items[about_index].label, "About Koushi");

    let user_settings_index = items
        .iter()
        .position(|item| item.id == "open_user_settings")
        .expect("user settings menu item should exist");
    let sign_out_index = items
        .iter()
        .position(|item| item.id == "sign_out")
        .expect("sign out menu item should exist");
    assert_eq!(sign_out_index, user_settings_index + 1);
    assert!(items.iter().any(|item| {
        item.id == "show_help" && item.accelerator.is_empty() && item.menu == "help"
    }));
    assert!(items.iter().any(|item| {
        item.id == "toggle_right_panel" && item.accelerator == "CmdOrCtrl+." && item.menu == "view"
    }));

    #[cfg(target_os = "macos")]
    assert!(items.iter().any(|item| {
        item.id == "toggle_fullscreen"
            && item.accelerator == "Ctrl+Command+F"
            && item.menu == "view"
    }));
}

#[test]
fn desktop_menu_items_include_platform_standard_close_and_quit() {
    let items = desktop_standard_menu_items();

    assert!(items.iter().any(|item| {
        item.id == "close_window" && item.accelerator == "CmdOrCtrl+W" && item.menu == "file"
    }));
    assert!(items.iter().any(|item| {
        item.id == "quit" && item.accelerator == "CmdOrCtrl+Q" && item.menu == "app"
    }));
}

#[test]
fn close_to_hide_requires_both_the_setting_and_a_real_tray() {
    // Linux/Windows default: opted in with a tray present.
    assert_eq!(
        close_requested_action(true, true),
        CloseRequestedAction::HideToTray
    );
    // Opted out: the close must destroy the window as before.
    assert_eq!(
        close_requested_action(true, false),
        CloseRequestedAction::DestroyWindow
    );
    // No tray: hiding the only window would leave the process unreachable, so
    // the setting alone must never be enough.
    assert_eq!(
        close_requested_action(false, true),
        CloseRequestedAction::DestroyWindow
    );
    assert_eq!(
        close_requested_action(false, false),
        CloseRequestedAction::DestroyWindow
    );
}

#[test]
fn exit_request_shuts_core_down_exactly_once_before_exiting() {
    // First Quit holds the exit and submits shutdown.
    assert_eq!(
        quit_request_action(QuitStage::Idle),
        QuitRequestAction::BeginShutdown
    );
    // A second Quit while shutdown is in flight must not submit a second one.
    assert_eq!(
        quit_request_action(QuitStage::ShuttingDown),
        QuitRequestAction::AwaitShutdown
    );
    // The exit re-requested by the shutdown task proceeds.
    assert_eq!(
        quit_request_action(QuitStage::ShutdownComplete),
        QuitRequestAction::Exit
    );
}

#[test]
fn only_one_caller_claims_core_shutdown() {
    // The window-destroy path and the `ExitRequested` that follows it both try
    // to start shutdown; exactly one may submit `AppCommand::Shutdown`.
    let quit_stage = AtomicU8::new(QuitStage::Idle.repr());
    assert!(claim_core_shutdown(&quit_stage));
    assert_eq!(
        QuitStage::from_repr(quit_stage.load(Ordering::Acquire)),
        QuitStage::ShuttingDown
    );
    assert!(!claim_core_shutdown(&quit_stage));

    // A completed shutdown is never restarted by a late claim.
    let quit_stage = AtomicU8::new(QuitStage::ShutdownComplete.repr());
    assert!(!claim_core_shutdown(&quit_stage));
    assert_eq!(
        QuitStage::from_repr(quit_stage.load(Ordering::Acquire)),
        QuitStage::ShutdownComplete
    );
}

#[test]
fn quit_stage_survives_its_atomic_representation() {
    for stage in [
        QuitStage::Idle,
        QuitStage::ShuttingDown,
        QuitStage::ShutdownComplete,
    ] {
        assert_eq!(QuitStage::from_repr(stage.repr()), stage);
    }
    // An unexpected stored value must fail closed to "shutdown not started"
    // rather than letting the process exit without shutting core down.
    assert_eq!(QuitStage::from_repr(200), QuitStage::Idle);
}

#[test]
fn account_badges_aggregate_saturating_across_tabs() {
    assert_eq!(super::aggregate_account_badge_count([2, 4, 0]), 6);
    assert_eq!(
        super::aggregate_account_badge_count([u64::MAX, 1]),
        u64::MAX
    );
}

#[test]
fn account_tab_snapshot_projects_badges_from_every_runtime() {
    use koushi_core::account_runtime_manager::{AccountTabDescriptor, AccountTabId};
    use koushi_protocol::AccountKey;

    let descriptor = |id: &str, user_id: &str| AccountTabDescriptor {
        id: AccountTabId::from_string(id.to_owned()),
        account_key: Some(AccountKey(user_id.to_owned())),
        homeserver: Some("https://example.invalid".to_owned()),
    };
    let mut alice = koushi_state::AppState::default();
    alice.native_attention.summary.badge_count = 2;
    alice.native_attention.summary.unread_count = 1;
    let mut bob = koushi_state::AppState::default();
    bob.native_attention.summary.badge_count = 4;
    bob.native_attention.summary.unread_count = 3;

    let snapshot = super::account_tabs_snapshot_from_states(
        "account:@bob:example.invalid".to_owned(),
        [
            (
                descriptor("account:@alice:example.invalid", "@alice:example.invalid"),
                alice,
            ),
            (
                descriptor("account:@bob:example.invalid", "@bob:example.invalid"),
                bob,
            ),
        ],
    );

    assert_eq!(snapshot.selected_tab_id, "account:@bob:example.invalid");
    assert_eq!(snapshot.badge_count, 6);
    assert_eq!(snapshot.tabs[0].unread_count, 1);
    assert_eq!(snapshot.tabs[1].unread_count, 3);
}

#[tokio::test]
async fn selected_connection_waits_for_restore_before_caching_initial_runtime() {
    let data_dir = tempfile::tempdir().expect("data directory");
    let native_artifact_factory: std::sync::Arc<
        dyn Fn() -> std::sync::Arc<dyn koushi_core::NativeArtifactPort> + Send + Sync,
    > = std::sync::Arc::new(|| std::sync::Arc::new(koushi_core::NativeArtifactRegistry::new()));
    let runtime = std::sync::Arc::new(
        koushi_core::account_runtime_manager::AccountRuntimeManager::new(
            koushi_core::store::StoreActor::with_backend(
                koushi_core::store::TestCredentialStoreBackend::in_memory(),
                data_dir.path(),
            ),
            koushi_core::settings::SettingsStore::new(data_dir.path()),
            native_artifact_factory,
        ),
    );
    let connections: super::AccountConnections =
        std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
    let (restore_ready_sender, restore_ready) = tokio::sync::watch::channel(false);
    let connection = super::SelectedCoreConnection {
        runtime: std::sync::Arc::clone(&runtime),
        connections,
        restore_ready,
    };

    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(10), connection.lock())
            .await
            .is_err(),
        "commands must not cache a connection while startup restore may replace it"
    );
    runtime
        .restore_saved_accounts()
        .await
        .expect("restore the empty account index");
    restore_ready_sender
        .send(true)
        .expect("startup restore gate should still have a receiver");
    let connection_guard =
        tokio::time::timeout(std::time::Duration::from_secs(1), connection.lock())
            .await
            .expect("commands should proceed after restore");
    drop(connection_guard);
    connection.clear_cached_connections().await;
    runtime.shutdown_all().await;
}

#[tokio::test]
async fn account_ipc_connection_stays_bound_when_selected_tab_changes() {
    let data_dir = tempfile::tempdir().expect("data directory");
    let native_artifact_factory: std::sync::Arc<
        dyn Fn() -> std::sync::Arc<dyn koushi_core::NativeArtifactPort> + Send + Sync,
    > = std::sync::Arc::new(|| std::sync::Arc::new(koushi_core::NativeArtifactRegistry::new()));
    let runtime = std::sync::Arc::new(
        koushi_core::account_runtime_manager::AccountRuntimeManager::new(
            koushi_core::store::StoreActor::with_backend(
                koushi_core::store::TestCredentialStoreBackend::in_memory(),
                data_dir.path(),
            ),
            koushi_core::settings::SettingsStore::new(data_dir.path()),
            native_artifact_factory,
        ),
    );
    let tab_a = runtime.selected_tab_id();
    assert!(matches!(
        runtime
            .bind_authenticated_session(
                &tab_a,
                &koushi_state::SessionInfo {
                    homeserver: "https://alice.example".to_owned(),
                    user_id: "@alice:example".to_owned(),
                    device_id: "ALICEDEVICE".to_owned(),
                    authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
                },
            )
            .await
            .expect("bind test account A"),
        koushi_core::account_runtime_manager::BindAccountOutcome::Bound(_)
    ));
    let tab_b = runtime.add_account_tab().await.expect("add account tab");
    assert_ne!(tab_a, tab_b);
    let connections: super::AccountConnections =
        std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
    let (_, restore_ready) = tokio::sync::watch::channel(true);
    let state = super::CoreRuntimeState {
        runtime: std::sync::Arc::clone(&runtime),
        connection: super::SelectedCoreConnection {
            runtime: std::sync::Arc::clone(&runtime),
            connections: std::sync::Arc::clone(&connections),
            restore_ready: restore_ready.clone(),
        },
        #[cfg(not(target_os = "macos"))]
        window_lifecycle_connection: super::SelectedCoreConnection {
            runtime: std::sync::Arc::clone(&runtime),
            connections,
            restore_ready,
        },
        timeline_items_count: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        _forwarder_task: std::sync::Mutex::new(None),
        startup_restore_task: std::sync::Mutex::new(None),
        account_tab_watchers: tokio::sync::Mutex::new(Vec::new()),
        native_window_focused: std::sync::atomic::AtomicBool::new(false),
        native_window_focus_generation: AtomicU64::new(0),
        viewport_sync_generation: super::viewport_sync::ViewportSyncGeneration::default(),
        quit_stage: AtomicU8::new(QuitStage::Idle.repr()),
        restart_after_shutdown: std::sync::atomic::AtomicBool::new(false),
        reader_subscriptions: tokio::sync::Mutex::new(std::collections::HashMap::new()),
    };

    let b = crate::commands::account_connection(&state, Some(tab_b.as_str()))
        .await
        .expect("resolve account B connection");
    let a = crate::commands::account_connection(&state, Some(tab_a.as_str()))
        .await
        .expect("resolve account A connection");
    let b_request_id = crate::commands::next_request_id_for(&state, Some(tab_b.as_str()))
        .await
        .expect("allocate account B request id");
    let a_request_id = crate::commands::next_request_id_for(&state, Some(tab_a.as_str()))
        .await
        .expect("allocate account A request id");
    assert_ne!(
        a_request_id.connection_id, b_request_id.connection_id,
        "connection ids must be unique across account runtimes"
    );
    crate::commands::submit_core_command_with_admission(
        &state,
        crate::commands::native_attention::build_observe_native_window_focus_command(
            b_request_id,
            true,
            1,
        ),
    )
    .await
    .expect("submit account B focus update through the shared dispatcher");
    crate::commands::submit_core_command_with_admission(
        &state,
        crate::commands::native_attention::build_observe_native_window_focus_command(
            a_request_id,
            false,
            1,
        ),
    )
    .await
    .expect("submit account A focus update through the shared dispatcher");

    assert_eq!(runtime.selected_tab_id(), tab_b);
    let context_a = runtime
        .tab_connection(&tab_a)
        .expect("account A connection")
        .snapshot()
        .native_attention_context;
    let context_b = runtime
        .tab_connection(&tab_b)
        .expect("account B connection")
        .snapshot()
        .native_attention_context;
    assert!(
        !context_a.window_focused,
        "account A context: {context_a:?}"
    );
    assert!(context_b.window_focused, "account B context: {context_b:?}");

    assert!(
        runtime
            .select_tab(&tab_a)
            .await
            .expect("select account A tab")
    );
    let focus_generation = AtomicU64::new(1);
    crate::commands::native_attention::transfer_native_window_focus(
        &runtime,
        &focus_generation,
        &tab_b,
        &tab_a,
        true,
    )
    .await;
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            let a_focused = runtime
                .tab_connection(&tab_a)
                .expect("account A connection")
                .snapshot()
                .native_attention_context
                .window_focused;
            let b_focused = runtime
                .tab_connection(&tab_b)
                .expect("account B connection")
                .snapshot()
                .native_attention_context
                .window_focused;
            if a_focused && !b_focused {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("tab selection transfers native focus between account runtimes");

    drop((a, b));
    assert_eq!(
        super::stop_core_state_for_exit(&state).await,
        super::CoreExitOutcome::Completed
    );
}

#[tokio::test]
async fn application_shutdown_releases_cached_account_connections() {
    let data_dir = tempfile::tempdir().expect("data directory");
    koushi_core::settings::SettingsStore::new(data_dir.path())
        .save_patch(&koushi_state::SettingsPatch {
            scope: Some(koushi_state::SettingsPatchScope::App),
            window: Some(koushi_state::WindowSettings {
                close_to_tray: false,
            }),
            ..Default::default()
        })
        .expect("persist app-owned close preference");
    let native_artifact_factory: std::sync::Arc<
        dyn Fn() -> std::sync::Arc<dyn koushi_core::NativeArtifactPort> + Send + Sync,
    > = std::sync::Arc::new(|| std::sync::Arc::new(koushi_core::NativeArtifactRegistry::new()));
    let runtime = std::sync::Arc::new(
        koushi_core::account_runtime_manager::AccountRuntimeManager::new(
            koushi_core::store::StoreActor::with_backend(
                koushi_core::store::TestCredentialStoreBackend::in_memory(),
                data_dir.path(),
            ),
            koushi_core::settings::SettingsStore::new(data_dir.path()),
            native_artifact_factory,
        ),
    );
    let connections: super::AccountConnections =
        std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
    let (_, restore_ready) = tokio::sync::watch::channel(true);
    let (release_restore, wait_for_restore) = tokio::sync::oneshot::channel();
    let restore_task = tauri::async_runtime::spawn(async move {
        wait_for_restore.await.expect("release startup restore");
    });
    let state = super::CoreRuntimeState {
        runtime: std::sync::Arc::clone(&runtime),
        connection: super::SelectedCoreConnection {
            runtime: std::sync::Arc::clone(&runtime),
            connections: std::sync::Arc::clone(&connections),
            restore_ready: restore_ready.clone(),
        },
        #[cfg(not(target_os = "macos"))]
        window_lifecycle_connection: super::SelectedCoreConnection {
            runtime: std::sync::Arc::clone(&runtime),
            connections,
            restore_ready,
        },
        timeline_items_count: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        _forwarder_task: std::sync::Mutex::new(None),
        startup_restore_task: std::sync::Mutex::new(Some(restore_task)),
        account_tab_watchers: tokio::sync::Mutex::new(Vec::new()),

        native_window_focused: std::sync::atomic::AtomicBool::new(false),
        native_window_focus_generation: AtomicU64::new(0),
        viewport_sync_generation: super::viewport_sync::ViewportSyncGeneration::default(),
        quit_stage: AtomicU8::new(QuitStage::Idle.repr()),
        restart_after_shutdown: std::sync::atomic::AtomicBool::new(false),
        reader_subscriptions: tokio::sync::Mutex::new(std::collections::HashMap::new()),
    };
    let tab_id = runtime.selected_tab_id();
    drop(
        state
            .connection
            .lock_for_tab_id(&tab_id)
            .await
            .expect("cache selected account connection"),
    );

    let mut shutdown = Box::pin(super::stop_core_state_for_exit(&state));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), &mut shutdown)
            .await
            .is_err(),
        "app shutdown must await startup restore"
    );
    release_restore.send(()).expect("release startup restore");
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(3), shutdown)
        .await
        .expect("app shutdown should release owned connections");
    assert_eq!(outcome, super::CoreExitOutcome::Completed);
    #[cfg(not(target_os = "macos"))]
    assert!(
        !state
            .window_lifecycle_connection
            .window_settings()
            .close_to_tray
    );
}
