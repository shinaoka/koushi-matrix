# Issue #551 Tauri Window-state Owner

## Scope

Move the complete logical window geometry/persistence owner and its deterministic tests from `apps/desktop/src-tauri/src/lib.rs` to private `src/window_state.rs`. Keep native visibility/reopen, macOS close-to-background policy, diagnostics, runtime shutdown, bootstrap, and command registry in `lib.rs`.

Baseline `e3a8a5d3`: `lib.rs` 4,523 lines / 183,150 bytes / SHA-256 `fb68bed06153f4c0b42e5d30b503432258339e97f580d83b0c184b4f097d2044`.

## Exact production ownership

Move five constants:

- `MIN_RESTORABLE_WINDOW_WIDTH`, `MIN_RESTORABLE_WINDOW_HEIGHT`
- `DEFAULT_WINDOW_WIDTH_LOGICAL`, `DEFAULT_WINDOW_HEIGHT_LOGICAL`
- `WINDOW_STATE_SCHEMA_VERSION`

Move seven types and the gate impl:

- `PersistedWindowState`
- `AppliedWindowGeometry`
- `WindowStatePersistenceAction`
- `WindowStatePersistencePhase`
- `WindowCloseEvent`
- `WindowStatePersistenceGate` and its `phase/arm/observe/is_ready` impl
- `WindowWorkArea`

Move these28 functions exactly:

- `window_close_should_persist`, `window_state_path`, `valid_window_scale_factor`, `capture_window_geometry`, `physical_size_for_logical_size`, `max_logical_dimension`, `max_logical_size_for_work_area`;
- `persisted_window_state_is_restorable`, `rectangle_intersection_area`, `clamp_physical_position`, `window_work_area_is_usable`, `selected_work_area`, `clamped_logical_size`, `restored_window_geometry`, `default_window_geometry`, `persisted_window_state_from_geometry`;
- `window_event_is_geometry`, `window_event_should_persist`, `load_window_state_with_base`, `load_window_state`, `persist_window_state_with_base`, `persist_window_state`, `apply_persisted_window_state`, `restore_main_window_state`, `persisted_window_state_from_window`, `persist_current_window_state`, `persist_observed_window_geometry`, `persist_close_window_state_if_ready`.

Preserve bodies, attrs, schema/version, path, atomic tmp+rename behavior, fail-closed pre-arm gate and all geometry math.

The leaf imports `std::{fs, path::{Path, PathBuf}, sync::Mutex}`, `serde::{Deserialize, Serialize}`, Tauri `Manager` plus exact geometry/window types, and `crate::app_data_dir`; tests use the existing `tempfile` dev dependency. Parent has one unconditional import group with exactly seven `pub(super)` items: `WindowStatePersistenceGate`, `WindowCloseEvent`, `restore_main_window_state`, `persist_close_window_state_if_ready`, `persist_observed_window_geometry`, `window_event_should_persist`, and `window_event_is_geometry`. Tests move with the owner, so test-only geometry internals do not become parent-visible. No façade/re-export/glob.

## Tests

Move these22 tests exactly:

- `window_state_path_is_separate_from_encrypted_session_stores`, `window_state_v2_json_round_trips_and_legacy_json_is_rejected`, `legacy_physical_capture_at_two_x_fails_logical_minimum`, `capture_preserves_logical_size_across_one_x_two_x_and_fractional_scales`, `mixed_dpi_restore_selects_physical_monitor_and_clamps_target_size`, `default_window_geometry_centers_with_floor_for_odd_physical_slack`;
- `window_state_gate_suppresses_prearm_and_all_initial_expected_cross_product_echoes`, `window_state_gate_retires_immediately_for_user_geometry_difference_without_ack`, `window_state_gate_suppresses_maximize_echo_then_persists_user_unmaximize`, `close_and_destroyed_persist_only_after_ready_gate`;
- `persisted_window_state_rejects_tiny_or_empty_geometry`, `window_state_persistence_writes_json_atomically_to_app_shell_path`, `window_state_load_ignores_corrupted_or_unrestorable_json`, `persisted_window_state_from_geometry_preserves_position_size_and_maximized_flag`;
- `restored_window_geometry_preserves_valid_in_bounds_state`, `restored_window_geometry_clamps_large_logical_state_to_work_area`, `restored_window_geometry_recovers_wholly_off_screen_state_to_primary`, `restored_window_geometry_uses_primary_after_secondary_monitor_disconnect`, `restored_window_geometry_preserves_valid_negative_monitor_coordinates`, `restored_window_geometry_rejects_work_area_smaller_than_minimum_window`, `restored_window_geometry_skips_intersecting_unusable_work_area`, `window_event_should_persist_for_geometry_changes_but_not_focus`.

Move private helpers `persisted_v2`, `scaled_work_area`, `geometry`, and `work_area`. Keep focus generation, shutdown, macOS hide/fullscreen, single-instance and reopen tests in `lib.rs`.

Trim the17 moved items from the parent test import list. Change the macOS hide source-test end marker from moved `fn load_window_state_with_base` to retained `fn ensure_main_window_visible`; preserve its hide-before-shutdown assertion and the unchanged `on_window_event` call order. No compatibility wrapper remains in `lib.rs`.

## Boundaries retained in `lib.rs`

- `ensure_main_window_visible*`, native macOS activation/order functions and QA visibility mode;
- `MacosCloseRequestedAction`, its diagnostic token, close-to-hide/fullscreen policy;
- `window_event_should_stop_background_tasks` and `submit_core_shutdown`;
- `run()`, managed state, forwarder/OIDC, registry183.

Replace the exact #544 plan sentence `Keep code in apps/desktop/src-tauri/src/lib.rs...` with the private `window_state.rs` owner sentence already staged in this change; behavior rules remain unchanged.

## Deterministic verifier

- constants5, types7, gate impl/methods4, production functions exact by kind/name/body;
- moved tests22 and helpers exact; parent definitions0;
- expected parent imports/calls only; no public wire/API/resource delta;
- native visibility/macOS close/run/registry183/`serialize_core_event` exact;
- no glob/default export, second state file, task, timer or dependency.

Run window-state focused baseline/post x3, full Tauri, mac GUI source contracts, full local matrix, design/full-diff review, latest-main integration, CI7/7 and #551 evidence.

## Gates

- `reviewer-flash` design verdict: `Correct-to-implement`; all inventory, baseline, import, #544 and source-test precision findings incorporated.
- move-only exactness and focused checks.
- `reviewer-flash` full-diff `Correct-to-merge`.
- merge and cleanup.
