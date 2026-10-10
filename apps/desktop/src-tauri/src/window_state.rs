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
// v3 captures the inner size that `set_size` restores; v2 captured the outer
// frame, so every launch grew the window by its decorations on Linux.
const WINDOW_STATE_SCHEMA_VERSION: u8 = 3;
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
#[derive(Clone, Copy, Debug, PartialEq)]
struct WindowFrameSample {
    outer_position: tauri::PhysicalPosition<i32>,
    inner_size: tauri::PhysicalSize<u32>,
    outer_size: tauri::PhysicalSize<u32>,
    scale_factor: f64,
    maximized: bool,
}
impl WindowFrameSample {
    fn capture<R: tauri::Runtime>(window: &tauri::Window<R>) -> Result<Self, String> {
        Ok(Self {
            outer_position: window
                .outer_position()
                .map_err(|_| "window position could not be captured".to_owned())?,
            inner_size: window
                .inner_size()
                .map_err(|_| "window size could not be captured".to_owned())?,
            outer_size: window
                .outer_size()
                .map_err(|_| "window frame size could not be captured".to_owned())?,
            scale_factor: window
                .scale_factor()
                .map_err(|_| "window scale factor could not be captured".to_owned())?,
            maximized: window
                .is_maximized()
                .map_err(|_| "window maximized state could not be captured".to_owned())?,
        })
    }

    fn geometry(&self) -> AppliedWindowGeometry {
        capture_window_geometry(
            self.outer_position,
            self.inner_size,
            self.scale_factor,
            self.maximized,
        )
    }

    fn persisted_state(&self) -> PersistedWindowState {
        persisted_window_state_from_geometry(
            self.outer_position,
            self.inner_size,
            self.scale_factor,
            self.maximized,
        )
    }

    /// Title bar, borders, and (on GTK) client-side shadow around the inner
    /// size. GTK reports a bogus outer size until the first configure event,
    /// so a frame smaller than its content counts as undecorated.
    fn decoration_logical(&self) -> tauri::LogicalSize<u32> {
        if !valid_window_scale_factor(self.scale_factor) {
            return tauri::LogicalSize::new(0, 0);
        }
        tauri::PhysicalSize::new(
            self.outer_size.width.saturating_sub(self.inner_size.width),
            self.outer_size
                .height
                .saturating_sub(self.inner_size.height),
        )
        .to_logical::<u32>(self.scale_factor)
    }
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
            let size_matches = current.logical_size == initial.logical_size
                || current.logical_size == expected.logical_size;
            let position_matches = current.physical_position == initial.physical_position
                || current.physical_position == expected.physical_position;
            // Native maximize can finish without a geometry event reporting
            // `maximized`. Keep startup echoes fenced, but let a later geometry
            // outside that finite fence retire it.
            if *expected_maximized_observed || !size_matches || !position_matches {
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
fn selected_work_area(
    x: i32,
    y: i32,
    size: tauri::PhysicalSize<u32>,
    work_areas: &[WindowWorkArea],
) -> Option<&WindowWorkArea> {
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
    decoration: tauri::LogicalSize<u32>,
) -> tauri::LogicalSize<u32> {
    let maximum = max_logical_size_for_work_area(area);
    let maximum_width = maximum
        .width
        .saturating_sub(decoration.width)
        .max(MIN_RESTORABLE_WINDOW_WIDTH);
    let maximum_height = maximum
        .height
        .saturating_sub(decoration.height)
        .max(MIN_RESTORABLE_WINDOW_HEIGHT);
    tauri::LogicalSize::new(
        logical_size.width.min(maximum_width),
        logical_size.height.min(maximum_height),
    )
}
fn outer_physical_size(
    logical_size: tauri::LogicalSize<u32>,
    decoration: tauri::LogicalSize<u32>,
    scale_factor: f64,
) -> tauri::PhysicalSize<u32> {
    physical_size_for_logical_size(
        tauri::LogicalSize::new(
            logical_size.width.saturating_add(decoration.width),
            logical_size.height.saturating_add(decoration.height),
        ),
        scale_factor,
    )
}
fn restored_window_geometry(
    state: &PersistedWindowState,
    work_areas: &[WindowWorkArea],
    decoration: tauri::LogicalSize<u32>,
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
        decoration,
    );
    let physical_size = outer_physical_size(logical_size, decoration, selected.scale_factor);
    let maximum_x =
        i64::from(selected.x) + i64::from(selected.width.saturating_sub(physical_size.width));
    let maximum_y =
        i64::from(selected.y) + i64::from(selected.height.saturating_sub(physical_size.height));

    Some(AppliedWindowGeometry {
        logical_size,
        physical_position: tauri::PhysicalPosition::new(
            clamp_physical_position(state.x_physical, selected.x, maximum_x),
            clamp_physical_position(state.y_physical, selected.y, maximum_y),
        ),
        maximized: state.maximized,
    })
}
fn default_window_geometry(
    work_areas: &[WindowWorkArea],
    decoration: tauri::LogicalSize<u32>,
) -> Option<AppliedWindowGeometry> {
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
        decoration,
    );
    let physical_size = outer_physical_size(logical_size, decoration, selected.scale_factor);
    let x =
        i64::from(selected.x) + i64::from(selected.width.saturating_sub(physical_size.width)) / 2;
    let y =
        i64::from(selected.y) + i64::from(selected.height.saturating_sub(physical_size.height)) / 2;

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
    window: &tauri::Window<R>,
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
    let initial_sample = WindowFrameSample::capture(window)?;
    let decoration = initial_sample.decoration_logical();
    let expected = state
        .as_ref()
        .and_then(|state| restored_window_geometry(state, &work_areas, decoration))
        .or_else(|| default_window_geometry(&work_areas, decoration));
    let Some(expected) = expected else {
        return Ok(());
    };

    let initial = initial_sample.geometry();
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
    apply_persisted_window_state(
        &window.as_ref().window(),
        load_window_state()?,
        gate.inner(),
    )
}
fn persist_current_window_state<R: tauri::Runtime>(
    window: &tauri::Window<R>,
) -> Result<(), String> {
    persist_window_state(&WindowFrameSample::capture(window)?.persisted_state())
}
pub(super) fn persist_observed_window_geometry<R: tauri::Runtime>(
    window: &tauri::Window<R>,
) -> Result<(), String> {
    let Some(gate) = window.try_state::<Mutex<WindowStatePersistenceGate>>() else {
        return Ok(());
    };
    let sample = WindowFrameSample::capture(window)?;
    let action = gate
        .lock()
        .map_err(|_| "window state gate is unavailable".to_owned())?
        .observe(sample.geometry());
    if action == WindowStatePersistenceAction::Persist {
        persist_window_state(&sample.persisted_state())?;
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
mod tests;
