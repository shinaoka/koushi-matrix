use std::path::Path;

use super::{
    AppliedWindowGeometry, PersistedWindowState, WINDOW_STATE_SCHEMA_VERSION, WindowCloseEvent,
    WindowFrameSample, WindowStatePersistenceAction, WindowStatePersistenceGate,
    WindowStatePersistencePhase, WindowWorkArea, capture_window_geometry, default_window_geometry,
    load_window_state_with_base, persist_window_state_with_base,
    persisted_window_state_from_geometry, persisted_window_state_is_restorable,
    restored_window_geometry, window_close_should_persist, window_event_should_persist,
    window_state_path,
};

const NO_DECORATION: tauri::LogicalSize<u32> = tauri::LogicalSize {
    width: 0,
    height: 0,
};

#[test]
fn window_state_path_is_separate_from_encrypted_session_stores() {
    let path = window_state_path(Path::new("/tmp/koushi-desktop"));

    assert_eq!(
        path,
        Path::new("/tmp/koushi-desktop")
            .join("app-shell")
            .join("window-state.json")
    );
}
fn persisted_state(
    x_physical: i32,
    y_physical: i32,
    width_logical: u32,
    height_logical: u32,
    capture_scale_factor: f64,
    maximized: bool,
) -> PersistedWindowState {
    PersistedWindowState {
        version: WINDOW_STATE_SCHEMA_VERSION,
        x_physical,
        y_physical,
        width_logical,
        height_logical,
        capture_scale_factor,
        maximized,
    }
}
fn scaled_work_area(
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale_factor: f64,
    primary: bool,
) -> WindowWorkArea {
    WindowWorkArea {
        x,
        y,
        width,
        height,
        scale_factor,
        primary,
    }
}
fn geometry(
    x: i32,
    y: i32,
    width_physical: u32,
    height_physical: u32,
    scale_factor: f64,
    maximized: bool,
) -> AppliedWindowGeometry {
    capture_window_geometry(
        tauri::PhysicalPosition::new(x, y),
        tauri::PhysicalSize::new(width_physical, height_physical),
        scale_factor,
        maximized,
    )
}
#[test]
fn window_state_json_round_trips_and_legacy_json_is_rejected() {
    let tempdir = tempfile::tempdir().expect("tempdir should be created");
    let path = window_state_path(tempdir.path());
    std::fs::create_dir_all(path.parent().expect("state path should have parent"))
        .expect("state dir should be created");
    let state = persisted_state(24, 48, 1280, 820, 1.25, true);
    let json = serde_json::to_string(&state).expect("v2 state should serialize");

    assert_eq!(
        serde_json::from_str::<PersistedWindowState>(&json).expect("v2 JSON should parse"),
        state
    );
    assert_eq!(
        load_window_state_with_base(tempdir.path()).expect("missing state"),
        None
    );

    std::fs::write(&path, &json).expect("v2 state should be written");
    assert_eq!(
        load_window_state_with_base(tempdir.path()).expect("v2 state should load"),
        Some(state)
    );

    std::fs::write(
        &path,
        r#"{"x":24,"y":48,"width":1077,"height":853,"maximized":false}"#,
    )
    .expect("legacy state should be written");
    assert_eq!(
        load_window_state_with_base(tempdir.path()).expect("legacy state should be ignored"),
        None
    );
}
#[test]
fn legacy_physical_capture_at_two_x_fails_logical_minimum() {
    let captured = persisted_window_state_from_geometry(
        tauri::PhysicalPosition::new(10, 20),
        tauri::PhysicalSize::new(1077, 853),
        2.0,
        false,
    );

    assert_eq!(captured.width_logical, 539);
    assert_eq!(captured.height_logical, 427);
    assert!(!persisted_window_state_is_restorable(&captured));
}
#[test]
fn capture_preserves_logical_size_across_one_x_two_x_and_fractional_scales() {
    let one_x = persisted_window_state_from_geometry(
        tauri::PhysicalPosition::new(50, 70),
        tauri::PhysicalSize::new(1280, 820),
        1.0,
        false,
    );
    let two_x = persisted_window_state_from_geometry(
        tauri::PhysicalPosition::new(50, 70),
        tauri::PhysicalSize::new(2560, 1640),
        2.0,
        false,
    );
    assert_eq!(
        (one_x.width_logical, one_x.height_logical),
        (two_x.width_logical, two_x.height_logical)
    );

    let one_point_twenty_five = geometry(0, 0, 1573, 1029, 1.25, false);
    let one_point_five = geometry(0, 0, 1573, 1029, 1.5, false);
    assert_eq!(
        one_point_twenty_five.logical_size,
        tauri::LogicalSize::new(1258, 823)
    );
    assert_eq!(
        one_point_five.logical_size,
        tauri::LogicalSize::new(1049, 686)
    );
}
#[test]
fn mixed_dpi_restore_selects_physical_monitor_and_clamps_target_size() {
    let state = persisted_state(2200, 100, 1600, 900, 2.0, false);
    let restored = restored_window_geometry(
        &state,
        &[
            scaled_work_area(0, 0, 1920, 1080, 1.0, true),
            scaled_work_area(1920, 0, 2560, 1700, 2.0, false),
        ],
        NO_DECORATION,
    )
    .expect("a physical monitor should be selected");

    assert_eq!(
        restored.physical_position,
        tauri::PhysicalPosition::new(1920, 0)
    );
    assert_eq!(restored.logical_size, tauri::LogicalSize::new(1280, 850));
    assert!(!restored.maximized);
}
#[test]
fn default_window_geometry_centers_with_floor_for_odd_physical_slack() {
    let restored = default_window_geometry(
        &[scaled_work_area(11, 7, 1921, 1041, 1.0, true)],
        NO_DECORATION,
    )
    .expect("primary work area should be usable");

    assert_eq!(restored.logical_size, tauri::LogicalSize::new(1280, 820));
    assert_eq!(
        restored.physical_position,
        tauri::PhysicalPosition::new(331, 117)
    );
    assert!(!restored.maximized);
}
#[test]
fn window_state_gate_suppresses_prearm_and_all_initial_expected_cross_product_echoes() {
    let initial = geometry(10, 20, 1280, 820, 1.0, false);
    let expected = geometry(40, 50, 1280, 820, 1.0, false);
    let mut gate = WindowStatePersistenceGate::PreArm;

    assert_eq!(
        gate.observe(initial),
        WindowStatePersistenceAction::Suppress
    );
    gate.arm(initial, expected);

    for logical_size in [initial.logical_size, expected.logical_size] {
        for physical_position in [initial.physical_position, expected.physical_position] {
            let echo = AppliedWindowGeometry {
                logical_size,
                physical_position,
                maximized: false,
            };
            assert_eq!(gate.observe(echo), WindowStatePersistenceAction::Suppress);
            assert_eq!(gate.observe(echo), WindowStatePersistenceAction::Suppress);
        }
    }
    assert_eq!(gate.phase(), WindowStatePersistencePhase::Restoring);
}
#[test]
fn window_state_gate_retires_immediately_for_user_geometry_difference_without_ack() {
    let initial = geometry(10, 20, 1280, 820, 1.0, false);
    let expected = geometry(40, 50, 1280, 820, 1.0, false);
    let mut gate = WindowStatePersistenceGate::PreArm;
    gate.arm(initial, expected);

    let user_geometry = geometry(41, 50, 1280, 820, 1.0, false);
    assert_eq!(
        gate.observe(user_geometry),
        WindowStatePersistenceAction::Persist
    );
    assert_eq!(gate.phase(), WindowStatePersistencePhase::Ready);
}
#[test]
fn window_state_gate_suppresses_maximize_echo_then_persists_user_unmaximize() {
    let initial = geometry(10, 20, 1280, 820, 1.0, false);
    let expected = geometry(40, 50, 1280, 820, 1.0, true);
    let mut gate = WindowStatePersistenceGate::PreArm;
    gate.arm(initial, expected);

    let maximize_echo = geometry(0, 0, 1920, 1080, 1.0, true);
    assert_eq!(
        gate.observe(maximize_echo),
        WindowStatePersistenceAction::Suppress
    );
    assert_eq!(gate.phase(), WindowStatePersistencePhase::Restoring);

    let user_unmaximized = geometry(40, 50, 1280, 820, 1.0, false);
    assert_eq!(
        gate.observe(user_unmaximized),
        WindowStatePersistenceAction::Persist
    );
    assert_eq!(gate.phase(), WindowStatePersistencePhase::Ready);
}
#[test]
fn close_and_destroyed_persist_only_after_ready_gate() {
    let initial = geometry(10, 20, 1280, 820, 1.0, false);
    let mut gate = WindowStatePersistenceGate::PreArm;
    assert!(!window_close_should_persist(
        WindowCloseEvent::CloseRequested,
        &gate
    ));
    assert!(!window_close_should_persist(
        WindowCloseEvent::Destroyed,
        &gate
    ));
    gate.arm(initial, initial);
    assert!(!window_close_should_persist(
        WindowCloseEvent::CloseRequested,
        &gate
    ));
    assert!(!window_close_should_persist(
        WindowCloseEvent::Destroyed,
        &gate
    ));
    gate.observe(geometry(11, 20, 1280, 820, 1.0, false));
    assert!(window_close_should_persist(
        WindowCloseEvent::CloseRequested,
        &gate
    ));
    assert!(window_close_should_persist(
        WindowCloseEvent::Destroyed,
        &gate
    ));
}
#[test]
fn persisted_window_state_rejects_tiny_or_empty_geometry() {
    assert!(persisted_window_state_is_restorable(&persisted_state(
        20, 40, 1280, 820, 1.0, false
    )));
    assert!(!persisted_window_state_is_restorable(&persisted_state(
        20, 40, 120, 80, 1.0, false
    )));
    assert!(!persisted_window_state_is_restorable(&persisted_state(
        20, 40, 0, 820, 1.0, false
    )));
    assert!(!persisted_window_state_is_restorable(&persisted_state(
        20, 40, 1280, 820, 0.0, false
    )));
}
#[test]
fn window_state_persistence_writes_json_atomically_to_app_shell_path() {
    let tempdir = tempfile::tempdir().expect("tempdir should be created");
    let state = persisted_state(24, 48, 1440, 900, 1.0, true);

    persist_window_state_with_base(tempdir.path(), &state).expect("window state should be written");

    let saved = std::fs::read_to_string(window_state_path(tempdir.path()))
        .expect("window state json should be readable");
    assert!(saved.contains("\"width_logical\":1440"));
    assert!(saved.contains("\"maximized\":true"));
    assert!(!saved.contains("access_token"));
}
#[test]
fn window_state_load_ignores_corrupted_or_unrestorable_json() {
    let tempdir = tempfile::tempdir().expect("tempdir should be created");
    let path = window_state_path(tempdir.path());
    std::fs::create_dir_all(path.parent().expect("state path should have parent"))
        .expect("state dir should be created");

    std::fs::write(&path, b"{not-json").expect("corrupted state should be written");
    assert_eq!(
        load_window_state_with_base(tempdir.path()).expect("corruption should be ignored"),
        None
    );

    std::fs::write(
        &path,
        r#"{"x":1,"y":2,"width":300,"height":200,"maximized":false}"#,
    )
    .expect("legacy state should be written");
    assert_eq!(
        load_window_state_with_base(tempdir.path()).expect("legacy state should be ignored"),
        None
    );
}
#[test]
fn persisted_window_state_from_geometry_preserves_position_size_and_maximized_flag() {
    let state = persisted_window_state_from_geometry(
        tauri::PhysicalPosition::new(50, 70),
        tauri::PhysicalSize::new(1366, 768),
        1.0,
        true,
    );

    assert_eq!(state, persisted_state(50, 70, 1366, 768, 1.0, true));
}
fn work_area(x: i32, y: i32, width: u32, height: u32, primary: bool) -> WindowWorkArea {
    scaled_work_area(x, y, width, height, 1.0, primary)
}
#[test]
fn restored_window_geometry_preserves_valid_in_bounds_state() {
    let state = persisted_state(120, 80, 1280, 820, 1.0, false);

    assert_eq!(
        restored_window_geometry(&state, &[work_area(0, 0, 1920, 1040, true)], NO_DECORATION),
        Some(geometry(120, 80, 1280, 820, 1.0, false))
    );
}
#[test]
fn restored_window_geometry_clamps_large_logical_state_to_work_area() {
    let state = persisted_state(0, 52, 2624, 1644, 2.0, true);

    assert_eq!(
        restored_window_geometry(&state, &[work_area(0, 0, 1312, 848, true)], NO_DECORATION),
        Some(geometry(0, 0, 1312, 848, 1.0, true))
    );
}
#[test]
fn restored_window_geometry_recovers_wholly_off_screen_state_to_primary() {
    let state = persisted_state(5000, 3000, 1280, 820, 1.0, false);

    assert_eq!(
        restored_window_geometry(&state, &[work_area(0, 0, 1920, 1040, true)], NO_DECORATION),
        Some(geometry(640, 220, 1280, 820, 1.0, false))
    );
}
#[test]
fn restored_window_geometry_uses_primary_after_secondary_monitor_disconnect() {
    let state = persisted_state(2300, 140, 1000, 700, 1.0, false);

    assert_eq!(
        restored_window_geometry(
            &state,
            &[
                work_area(0, 0, 1920, 1040, true),
                work_area(-1600, 0, 1600, 900, false),
            ],
            NO_DECORATION,
        ),
        Some(geometry(920, 140, 1000, 700, 1.0, false))
    );
}
#[test]
fn restored_window_geometry_preserves_valid_negative_monitor_coordinates() {
    let state = persisted_state(-1800, -120, 1200, 800, 1.0, false);

    assert_eq!(
        restored_window_geometry(
            &state,
            &[
                work_area(0, 0, 1920, 1040, true),
                work_area(-1920, -200, 1920, 1080, false),
            ],
            NO_DECORATION,
        ),
        Some(geometry(-1800, -120, 1200, 800, 1.0, false))
    );
}
#[test]
fn restored_window_geometry_rejects_work_area_smaller_than_minimum_window() {
    let state = persisted_state(20, 20, 1280, 820, 1.0, false);

    assert_eq!(
        restored_window_geometry(&state, &[work_area(0, 0, 700, 600, true)], NO_DECORATION),
        None
    );
}
#[test]
fn restored_window_geometry_skips_intersecting_unusable_work_area() {
    let state = persisted_state(2050, 50, 1280, 820, 1.0, false);

    assert_eq!(
        restored_window_geometry(
            &state,
            &[
                work_area(0, 0, 1920, 1040, true),
                work_area(2000, 0, 700, 600, false),
            ],
            NO_DECORATION,
        ),
        Some(geometry(640, 50, 1280, 820, 1.0, false))
    );
}
#[test]
fn window_event_should_persist_for_geometry_changes_but_not_focus() {
    assert!(window_event_should_persist(&tauri::WindowEvent::Resized(
        tauri::PhysicalSize::new(1280, 820)
    )));
    assert!(window_event_should_persist(&tauri::WindowEvent::Moved(
        tauri::PhysicalPosition::new(30, 50)
    )));
    assert!(!window_event_should_persist(&tauri::WindowEvent::Focused(
        true
    )));
}
fn linux_csd_sample(inner_logical: (u32, u32), maximized: bool) -> WindowFrameSample {
    // GTK reports an integer scale of 2 under KDE Wayland's 1.6x fractional
    // scale, and its frame extents add the client-side title bar and shadow.
    let scale_factor = 2.0;
    WindowFrameSample {
        outer_position: tauri::PhysicalPosition::new(0, 0),
        inner_size: tauri::PhysicalSize::new(inner_logical.0 * 2, inner_logical.1 * 2),
        outer_size: tauri::PhysicalSize::new(
            (inner_logical.0 + 90) * 2,
            (inner_logical.1 + 138) * 2,
        ),
        scale_factor,
        maximized,
    }
}
#[test]
fn window_state_capture_is_stable_across_launches_with_decorations() {
    let work_areas = [scaled_work_area(0, 0, 3600, 2250, 2.0, true)];
    let first = linux_csd_sample((1280, 820), false);
    let mut state = first.persisted_state();

    for _ in 0..4 {
        let restored = restored_window_geometry(&state, &work_areas, first.decoration_logical())
            .expect("state should restore");
        let relaunched = linux_csd_sample(
            (restored.logical_size.width, restored.logical_size.height),
            false,
        );
        state = relaunched.persisted_state();
    }

    assert_eq!((state.width_logical, state.height_logical), (1280, 820));
}
#[test]
fn restored_window_geometry_reserves_decorations_inside_work_area() {
    let sample = linux_csd_sample((1280, 820), false);
    let state = persisted_state(0, 0, 1890, 1263, 2.0, false);

    let restored = restored_window_geometry(
        &state,
        &[scaled_work_area(0, 0, 3600, 2250, 2.0, true)],
        sample.decoration_logical(),
    )
    .expect("state should restore");

    assert_eq!(restored.logical_size, tauri::LogicalSize::new(1710, 987));
    assert_eq!(
        restored.physical_position,
        tauri::PhysicalPosition::new(0, 0)
    );
}
#[test]
fn default_window_geometry_centers_outer_frame_with_decorations() {
    let restored = default_window_geometry(
        &[scaled_work_area(0, 0, 1440, 900, 1.0, true)],
        tauri::LogicalSize::new(10, 40),
    )
    .expect("primary work area should be usable");

    assert_eq!(restored.logical_size, tauri::LogicalSize::new(1280, 820));
    assert_eq!(
        restored.physical_position,
        tauri::PhysicalPosition::new(75, 20)
    );
}
#[test]
fn window_frame_sample_ignores_outer_size_smaller_than_inner_size() {
    let sample = WindowFrameSample {
        outer_position: tauri::PhysicalPosition::new(0, 0),
        inner_size: tauri::PhysicalSize::new(1280, 820),
        outer_size: tauri::PhysicalSize::new(0, 0),
        scale_factor: 1.0,
        maximized: false,
    };

    assert_eq!(sample.decoration_logical(), NO_DECORATION);
}
