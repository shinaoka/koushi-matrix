use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::{Deserialize, Serialize};
use tauri::Manager;

use crate::app_data_dir;

const MIN_RESTORABLE_WINDOW_WIDTH: u32 = 760;
const MIN_RESTORABLE_WINDOW_HEIGHT: u32 = 620;
const DEFAULT_WINDOW_WIDTH_LOGICAL: u32 = 1280;
const DEFAULT_WINDOW_HEIGHT_LOGICAL: u32 = 820;
const WINDOW_STATE_SCHEMA_VERSION: u8 = 2;
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
struct PersistedWindowState {
    pub version: u8,
    pub x_physical: i32,
    pub y_physical: i32,
    pub width_logical: u32,
    pub height_logical: u32,
    pub capture_scale_factor: f64,
    pub maximized: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct AppliedWindowGeometry {
    logical_size: tauri::LogicalSize<u32>,
    physical_position: tauri::PhysicalPosition<i32>,
    maximized: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WindowStatePersistenceAction {
    Suppress,
    Persist,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WindowStatePersistencePhase {
    PreArm,
    Restoring,
    Ready,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WindowCloseEvent {
    CloseRequested,
    Destroyed,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WindowStatePersistenceGate {
    PreArm,
    Restoring {
        initial: AppliedWindowGeometry,
        expected: AppliedWindowGeometry,
        expected_maximized_observed: bool,
    },
    Ready,
}
impl WindowStatePersistenceGate {
    fn phase(self) -> WindowStatePersistencePhase {
        match self {
            Self::PreArm => WindowStatePersistencePhase::PreArm,
            Self::Restoring { .. } => WindowStatePersistencePhase::Restoring,
            Self::Ready => WindowStatePersistencePhase::Ready,
        }
    }

    fn arm(&mut self, initial: AppliedWindowGeometry, expected: AppliedWindowGeometry) {
        *self = Self::Restoring {
            initial,
            expected,
            expected_maximized_observed: initial.maximized == expected.maximized,
        };
    }

    fn observe(&mut self, current: AppliedWindowGeometry) -> WindowStatePersistenceAction {
        let Self::Restoring {
            initial,
            expected,
            ref mut expected_maximized_observed,
        } = *self
        else {
            return if self.phase() == WindowStatePersistencePhase::Ready {
                WindowStatePersistenceAction::Persist
            } else {
                WindowStatePersistenceAction::Suppress
            };
        };

        if expected.maximized {
            if current.maximized {
                *expected_maximized_observed = true;
                return WindowStatePersistenceAction::Suppress;
            }
            if *expected_maximized_observed {
                *self = Self::Ready;
                return WindowStatePersistenceAction::Persist;
            }
            return WindowStatePersistenceAction::Suppress;
        }

        let size_matches = current.logical_size == initial.logical_size
            || current.logical_size == expected.logical_size;
        let position_matches = current.physical_position == initial.physical_position
            || current.physical_position == expected.physical_position;
        if current.maximized || !size_matches || !position_matches {
            *self = Self::Ready;
            WindowStatePersistenceAction::Persist
        } else {
            WindowStatePersistenceAction::Suppress
        }
    }

    fn is_ready(self) -> bool {
        self.phase() == WindowStatePersistencePhase::Ready
    }
}
fn window_close_should_persist(
    _event: WindowCloseEvent,
    gate: &WindowStatePersistenceGate,
) -> bool {
    gate.is_ready()
}
fn window_state_path(base_dir: &Path) -> PathBuf {
    base_dir.join("app-shell").join("window-state.json")
}
fn valid_window_scale_factor(scale_factor: f64) -> bool {
    scale_factor.is_sign_positive() && scale_factor.is_normal()
}
fn capture_window_geometry(
    position: tauri::PhysicalPosition<i32>,
    size: tauri::PhysicalSize<u32>,
    scale_factor: f64,
    maximized: bool,
) -> AppliedWindowGeometry {
    AppliedWindowGeometry {
        logical_size: size.to_logical::<u32>(scale_factor),
        physical_position: position,
        maximized,
    }
}
fn physical_size_for_logical_size(
    logical_size: tauri::LogicalSize<u32>,
    scale_factor: f64,
) -> tauri::PhysicalSize<u32> {
    tauri::LogicalSize::new(
        f64::from(logical_size.width),
        f64::from(logical_size.height),
    )
    .to_physical::<u32>(scale_factor)
}
fn max_logical_dimension(physical: u32, scale_factor: f64) -> u32 {
    (f64::from(physical) / scale_factor).floor() as u32
}
fn max_logical_size_for_work_area(area: &WindowWorkArea) -> tauri::LogicalSize<u32> {
    tauri::LogicalSize::new(
        max_logical_dimension(area.width, area.scale_factor),
        max_logical_dimension(area.height, area.scale_factor),
    )
}
fn persisted_window_state_is_restorable(state: &PersistedWindowState) -> bool {
    state.version == WINDOW_STATE_SCHEMA_VERSION
        && state.width_logical >= MIN_RESTORABLE_WINDOW_WIDTH
        && state.height_logical >= MIN_RESTORABLE_WINDOW_HEIGHT
        && valid_window_scale_factor(state.capture_scale_factor)
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct WindowWorkArea {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale_factor: f64,
    primary: bool,
}
fn rectangle_intersection_area(
    x: i32,
    y: i32,
    size: tauri::PhysicalSize<u32>,
    area: &WindowWorkArea,
) -> u64 {
    let left = i64::from(x).max(i64::from(area.x));
    let top = i64::from(y).max(i64::from(area.y));
    let right =
        (i64::from(x) + i64::from(size.width)).min(i64::from(area.x) + i64::from(area.width));
    let bottom =
        (i64::from(y) + i64::from(size.height)).min(i64::from(area.y) + i64::from(area.height));

    let width = right.saturating_sub(left).max(0) as u64;
    let height = bottom.saturating_sub(top).max(0) as u64;
    width.saturating_mul(height)
}
fn clamp_physical_position(value: i32, minimum: i32, maximum: i64) -> i32 {
    i64::from(value).clamp(i64::from(minimum), maximum.min(i64::from(i32::MAX))) as i32
}
fn window_work_area_is_usable(area: &WindowWorkArea) -> bool {
    if !valid_window_scale_factor(area.scale_factor) {
        return false;
    }
    let maximum = max_logical_size_for_work_area(area);
    maximum.width >= MIN_RESTORABLE_WINDOW_WIDTH && maximum.height >= MIN_RESTORABLE_WINDOW_HEIGHT
}
fn selected_work_area<'a>(
    x: i32,
    y: i32,
    size: tauri::PhysicalSize<u32>,
    work_areas: &'a [WindowWorkArea],
) -> Option<&'a WindowWorkArea> {
    work_areas
        .iter()
        .filter(|area| window_work_area_is_usable(area))
        .map(|area| (area, rectangle_intersection_area(x, y, size, area)))
        .filter(|(_, intersection)| *intersection > 0)
        .max_by_key(|(_, intersection)| *intersection)
        .map(|(area, _)| area)
        .or_else(|| {
            work_areas
                .iter()
                .find(|area| area.primary && window_work_area_is_usable(area))
        })
        .or_else(|| {
            work_areas
                .iter()
                .find(|area| window_work_area_is_usable(area))
        })
}
fn clamped_logical_size(
    logical_size: tauri::LogicalSize<u32>,
    area: &WindowWorkArea,
) -> tauri::LogicalSize<u32> {
    let maximum = max_logical_size_for_work_area(area);
    tauri::LogicalSize::new(
        logical_size.width.min(maximum.width),
        logical_size.height.min(maximum.height),
    )
}
fn restored_window_geometry(
    state: &PersistedWindowState,
    work_areas: &[WindowWorkArea],
) -> Option<AppliedWindowGeometry> {
    if !persisted_window_state_is_restorable(state) {
        return None;
    }

    let saved_size = physical_size_for_logical_size(
        tauri::LogicalSize::new(state.width_logical, state.height_logical),
        state.capture_scale_factor,
    );
    let selected = selected_work_area(state.x_physical, state.y_physical, saved_size, work_areas)?;
    let logical_size = clamped_logical_size(
        tauri::LogicalSize::new(state.width_logical, state.height_logical),
        selected,
    );
    let physical_size = physical_size_for_logical_size(logical_size, selected.scale_factor);
    let maximum_x = i64::from(selected.x) + i64::from(selected.width - physical_size.width);
    let maximum_y = i64::from(selected.y) + i64::from(selected.height - physical_size.height);

    Some(AppliedWindowGeometry {
        logical_size,
        physical_position: tauri::PhysicalPosition::new(
            clamp_physical_position(state.x_physical, selected.x, maximum_x),
            clamp_physical_position(state.y_physical, selected.y, maximum_y),
        ),
        maximized: state.maximized,
    })
}
fn default_window_geometry(work_areas: &[WindowWorkArea]) -> Option<AppliedWindowGeometry> {
    let selected = work_areas
        .iter()
        .find(|area| area.primary && window_work_area_is_usable(area))
        .or_else(|| {
            work_areas
                .iter()
                .find(|area| window_work_area_is_usable(area))
        })?;
    let logical_size = clamped_logical_size(
        tauri::LogicalSize::new(DEFAULT_WINDOW_WIDTH_LOGICAL, DEFAULT_WINDOW_HEIGHT_LOGICAL),
        selected,
    );
    let physical_size = physical_size_for_logical_size(logical_size, selected.scale_factor);
    let x = i64::from(selected.x) + i64::from(selected.width - physical_size.width) / 2;
    let y = i64::from(selected.y) + i64::from(selected.height - physical_size.height) / 2;

    Some(AppliedWindowGeometry {
        logical_size,
        physical_position: tauri::PhysicalPosition::new(
            x.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
            y.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        ),
        maximized: false,
    })
}
fn persisted_window_state_from_geometry(
    position: tauri::PhysicalPosition<i32>,
    size: tauri::PhysicalSize<u32>,
    scale_factor: f64,
    maximized: bool,
) -> PersistedWindowState {
    let geometry = capture_window_geometry(position, size, scale_factor, maximized);
    PersistedWindowState {
        version: WINDOW_STATE_SCHEMA_VERSION,
        x_physical: geometry.physical_position.x,
        y_physical: geometry.physical_position.y,
        width_logical: geometry.logical_size.width,
        height_logical: geometry.logical_size.height,
        capture_scale_factor: scale_factor,
        maximized: geometry.maximized,
    }
}
pub(super) fn window_event_is_geometry(event: &tauri::WindowEvent) -> bool {
    matches!(
        event,
        tauri::WindowEvent::Resized(_)
            | tauri::WindowEvent::Moved(_)
            | tauri::WindowEvent::ScaleFactorChanged { .. }
    )
}
pub(super) fn window_event_should_persist(event: &tauri::WindowEvent) -> bool {
    window_event_is_geometry(event)
        || matches!(
            event,
            tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed
        )
}
fn load_window_state_with_base(base_dir: &Path) -> Result<Option<PersistedWindowState>, String> {
    let path = window_state_path(base_dir);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("window state could not be read".to_owned()),
    };

    let state = match serde_json::from_slice::<PersistedWindowState>(&bytes) {
        Ok(state) => state,
        Err(_) => return Ok(None),
    };

    Ok(persisted_window_state_is_restorable(&state).then_some(state))
}
fn load_window_state() -> Result<Option<PersistedWindowState>, String> {
    load_window_state_with_base(&app_data_dir()?)
}
fn persist_window_state_with_base(
    base_dir: &Path,
    state: &PersistedWindowState,
) -> Result<(), String> {
    if !persisted_window_state_is_restorable(state) {
        return Ok(());
    }

    let path = window_state_path(base_dir);
    let parent = path
        .parent()
        .ok_or_else(|| "window state path is invalid".to_owned())?;
    std::fs::create_dir_all(parent)
        .map_err(|_| "window state directory could not be created".to_owned())?;

    let tmp_path = parent.join("window-state.json.tmp");
    let json =
        serde_json::to_vec(state).map_err(|_| "window state could not be serialized".to_owned())?;
    std::fs::write(&tmp_path, json).map_err(|_| "window state could not be written".to_owned())?;
    std::fs::rename(&tmp_path, &path)
        .map_err(|_| "window state could not be committed".to_owned())?;
    Ok(())
}
fn persist_window_state(state: &PersistedWindowState) -> Result<(), String> {
    persist_window_state_with_base(&app_data_dir()?, state)
}
fn apply_persisted_window_state<R: tauri::Runtime>(
    window: &tauri::WebviewWindow<R>,
    state: Option<PersistedWindowState>,
    gate: &Mutex<WindowStatePersistenceGate>,
) -> Result<(), String> {
    let monitors = window
        .available_monitors()
        .map_err(|_| "active monitors could not be inspected".to_owned())?;
    let primary = window
        .primary_monitor()
        .map_err(|_| "primary monitor could not be inspected".to_owned())?;
    let work_areas = monitors
        .iter()
        .map(|monitor| {
            let work_area = monitor.work_area();
            let primary = primary.as_ref().is_some_and(|primary| {
                primary.position() == monitor.position()
                    && primary.size() == monitor.size()
                    && primary.work_area().position == monitor.work_area().position
                    && primary.work_area().size == monitor.work_area().size
            });
            WindowWorkArea {
                x: work_area.position.x,
                y: work_area.position.y,
                width: work_area.size.width,
                height: work_area.size.height,
                scale_factor: monitor.scale_factor(),
                primary,
            }
        })
        .collect::<Vec<_>>();
    let expected = state
        .as_ref()
        .and_then(|state| restored_window_geometry(state, &work_areas))
        .or_else(|| default_window_geometry(&work_areas));
    let Some(expected) = expected else {
        return Ok(());
    };

    let initial_position = window
        .outer_position()
        .map_err(|_| "window position could not be captured".to_owned())?;
    let initial_size = window
        .outer_size()
        .map_err(|_| "window size could not be captured".to_owned())?;
    let initial_scale_factor = window
        .scale_factor()
        .map_err(|_| "window scale factor could not be captured".to_owned())?;
    let initial_maximized = window
        .is_maximized()
        .map_err(|_| "window maximized state could not be captured".to_owned())?;
    let initial = capture_window_geometry(
        initial_position,
        initial_size,
        initial_scale_factor,
        initial_maximized,
    );
    gate.lock()
        .map_err(|_| "window state gate is unavailable".to_owned())?
        .arm(initial, expected);

    window
        .set_size(tauri::Size::Logical(tauri::LogicalSize::new(
            f64::from(expected.logical_size.width),
            f64::from(expected.logical_size.height),
        )))
        .map_err(|_| "window size could not be restored".to_owned())?;
    window
        .set_position(tauri::Position::Physical(expected.physical_position))
        .map_err(|_| "window position could not be restored".to_owned())?;
    if expected.maximized {
        window
            .maximize()
            .map_err(|_| "window maximized state could not be restored".to_owned())?;
    }
    Ok(())
}
pub(super) fn restore_main_window_state<R: tauri::Runtime, M: Manager<R>>(
    manager: &M,
) -> Result<(), String> {
    let Some(window) = manager.get_webview_window("main") else {
        return Ok(());
    };
    let Some(gate) = manager.try_state::<Mutex<WindowStatePersistenceGate>>() else {
        return Ok(());
    };
    apply_persisted_window_state(&window, load_window_state()?, gate.inner())
}
fn persisted_window_state_from_window<R: tauri::Runtime>(
    window: &tauri::Window<R>,
) -> Result<PersistedWindowState, String> {
    let position = window
        .outer_position()
        .map_err(|_| "window position could not be captured".to_owned())?;
    let size = window
        .outer_size()
        .map_err(|_| "window size could not be captured".to_owned())?;
    let scale_factor = window
        .scale_factor()
        .map_err(|_| "window scale factor could not be captured".to_owned())?;
    let maximized = window
        .is_maximized()
        .map_err(|_| "window maximized state could not be captured".to_owned())?;
    Ok(persisted_window_state_from_geometry(
        position,
        size,
        scale_factor,
        maximized,
    ))
}
fn persist_current_window_state<R: tauri::Runtime>(
    window: &tauri::Window<R>,
) -> Result<(), String> {
    let state = persisted_window_state_from_window(window)?;
    persist_window_state(&state)
}
pub(super) fn persist_observed_window_geometry<R: tauri::Runtime>(
    window: &tauri::Window<R>,
) -> Result<(), String> {
    let Some(gate) = window.try_state::<Mutex<WindowStatePersistenceGate>>() else {
        return Ok(());
    };
    let position = window
        .outer_position()
        .map_err(|_| "window position could not be captured".to_owned())?;
    let size = window
        .outer_size()
        .map_err(|_| "window size could not be captured".to_owned())?;
    let scale_factor = window
        .scale_factor()
        .map_err(|_| "window scale factor could not be captured".to_owned())?;
    let maximized = window
        .is_maximized()
        .map_err(|_| "window maximized state could not be captured".to_owned())?;
    let geometry = capture_window_geometry(position, size, scale_factor, maximized);
    let action = gate
        .lock()
        .map_err(|_| "window state gate is unavailable".to_owned())?
        .observe(geometry);
    if action == WindowStatePersistenceAction::Persist {
        persist_window_state(&persisted_window_state_from_geometry(
            position,
            size,
            scale_factor,
            maximized,
        ))?;
    }
    Ok(())
}
pub(super) fn persist_close_window_state_if_ready<R: tauri::Runtime>(
    window: &tauri::Window<R>,
    event: WindowCloseEvent,
) -> Result<(), String> {
    let Some(gate) = window.try_state::<Mutex<WindowStatePersistenceGate>>() else {
        return Ok(());
    };
    let should_persist = gate
        .lock()
        .map_err(|_| "window state gate is unavailable".to_owned())
        .map(|gate| window_close_should_persist(event, &gate))?;
    if should_persist {
        persist_current_window_state(window)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        AppliedWindowGeometry, PersistedWindowState, WindowCloseEvent,
        WindowStatePersistenceAction, WindowStatePersistenceGate, WindowStatePersistencePhase,
        WindowWorkArea, capture_window_geometry, default_window_geometry,
        load_window_state_with_base, persist_window_state_with_base,
        persisted_window_state_from_geometry, persisted_window_state_is_restorable,
        restored_window_geometry, window_close_should_persist, window_event_should_persist,
        window_state_path,
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
    fn persisted_v2(
        x_physical: i32,
        y_physical: i32,
        width_logical: u32,
        height_logical: u32,
        capture_scale_factor: f64,
        maximized: bool,
    ) -> PersistedWindowState {
        PersistedWindowState {
            version: 2,
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
    fn window_state_v2_json_round_trips_and_legacy_json_is_rejected() {
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        let path = window_state_path(tempdir.path());
        std::fs::create_dir_all(path.parent().expect("state path should have parent"))
            .expect("state dir should be created");
        let state = persisted_v2(24, 48, 1280, 820, 1.25, true);
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
        let state = persisted_v2(2200, 100, 1600, 900, 2.0, false);
        let restored = restored_window_geometry(
            &state,
            &[
                scaled_work_area(0, 0, 1920, 1080, 1.0, true),
                scaled_work_area(1920, 0, 2560, 1700, 2.0, false),
            ],
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
        let restored = default_window_geometry(&[scaled_work_area(11, 7, 1921, 1041, 1.0, true)])
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
        assert!(persisted_window_state_is_restorable(&persisted_v2(
            20, 40, 1280, 820, 1.0, false
        )));
        assert!(!persisted_window_state_is_restorable(&persisted_v2(
            20, 40, 120, 80, 1.0, false
        )));
        assert!(!persisted_window_state_is_restorable(&persisted_v2(
            20, 40, 0, 820, 1.0, false
        )));
        assert!(!persisted_window_state_is_restorable(&persisted_v2(
            20, 40, 1280, 820, 0.0, false
        )));
    }
    #[test]
    fn window_state_persistence_writes_json_atomically_to_app_shell_path() {
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        let state = persisted_v2(24, 48, 1440, 900, 1.0, true);

        persist_window_state_with_base(tempdir.path(), &state)
            .expect("window state should be written");

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

        assert_eq!(state, persisted_v2(50, 70, 1366, 768, 1.0, true));
    }
    fn work_area(x: i32, y: i32, width: u32, height: u32, primary: bool) -> WindowWorkArea {
        scaled_work_area(x, y, width, height, 1.0, primary)
    }
    #[test]
    fn restored_window_geometry_preserves_valid_in_bounds_state() {
        let state = persisted_v2(120, 80, 1280, 820, 1.0, false);

        assert_eq!(
            restored_window_geometry(&state, &[work_area(0, 0, 1920, 1040, true)]),
            Some(geometry(120, 80, 1280, 820, 1.0, false))
        );
    }
    #[test]
    fn restored_window_geometry_clamps_large_logical_state_to_work_area() {
        let state = persisted_v2(0, 52, 2624, 1644, 2.0, true);

        assert_eq!(
            restored_window_geometry(&state, &[work_area(0, 0, 1312, 848, true)]),
            Some(geometry(0, 0, 1312, 848, 1.0, true))
        );
    }
    #[test]
    fn restored_window_geometry_recovers_wholly_off_screen_state_to_primary() {
        let state = persisted_v2(5000, 3000, 1280, 820, 1.0, false);

        assert_eq!(
            restored_window_geometry(&state, &[work_area(0, 0, 1920, 1040, true)]),
            Some(geometry(640, 220, 1280, 820, 1.0, false))
        );
    }
    #[test]
    fn restored_window_geometry_uses_primary_after_secondary_monitor_disconnect() {
        let state = persisted_v2(2300, 140, 1000, 700, 1.0, false);

        assert_eq!(
            restored_window_geometry(
                &state,
                &[
                    work_area(0, 0, 1920, 1040, true),
                    work_area(-1600, 0, 1600, 900, false),
                ],
            ),
            Some(geometry(920, 140, 1000, 700, 1.0, false))
        );
    }
    #[test]
    fn restored_window_geometry_preserves_valid_negative_monitor_coordinates() {
        let state = persisted_v2(-1800, -120, 1200, 800, 1.0, false);

        assert_eq!(
            restored_window_geometry(
                &state,
                &[
                    work_area(0, 0, 1920, 1040, true),
                    work_area(-1920, -200, 1920, 1080, false),
                ],
            ),
            Some(geometry(-1800, -120, 1200, 800, 1.0, false))
        );
    }
    #[test]
    fn restored_window_geometry_rejects_work_area_smaller_than_minimum_window() {
        let state = persisted_v2(20, 20, 1280, 820, 1.0, false);

        assert_eq!(
            restored_window_geometry(&state, &[work_area(0, 0, 700, 600, true)]),
            None
        );
    }
    #[test]
    fn restored_window_geometry_skips_intersecting_unusable_work_area() {
        let state = persisted_v2(2050, 50, 1280, 820, 1.0, false);

        assert_eq!(
            restored_window_geometry(
                &state,
                &[
                    work_area(0, 0, 1920, 1040, true),
                    work_area(2000, 0, 700, 600, false),
                ],
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
}
