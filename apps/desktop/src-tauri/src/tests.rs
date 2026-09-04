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
fn renderable_asset_cache_scope_is_limited_to_media_cache_dirs() {
    let base = Path::new("/tmp/koushi-data");
    assert_eq!(
        super::renderable_asset_cache_dirs(base),
        [base.join("media_downloads")]
    );
}

#[test]
fn renderable_thumbnail_protocol_serves_known_cached_bytes() {
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
fn desktop_menu_items_include_element_compatible_shortcuts() {
    let items = desktop_menu_items();

    assert!(items.iter().any(|item| {
        item.id == "open_user_settings" && item.accelerator == "CmdOrCtrl+," && item.menu == "app"
    }));
    assert!(
        items
            .iter()
            .any(|item| item.id == "sign_out" && item.accelerator == "" && item.menu == "app")
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
        item.id == "show_keyboard_settings"
            && item.accelerator == "CmdOrCtrl+/"
            && item.menu == "help"
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
