# Issue #544 Logical Window-state Restore

## Scope

Replace ambiguous physical-pixel window persistence with a versioned logical-pixel schema and suppress programmatic startup geometry echoes. Preserve single-window ownership, maximized restore, atomic file writes, off-screen clamping, and user-initiated persistence. No frontend geometry owner, timer/sleep stabilization, platform-specific fork, or compatibility shim that keeps unsafe legacy geometry active.

## Persisted contract

`PersistedWindowState` becomes schema version 2 with an explicitly mixed but unit-consistent desktop contract:

- `x_physical` / `y_physical` are global physical desktop coordinates, because mixed-DPI monitors do not share one coherent global logical origin;
- `width_logical` / `height_logical` are integer logical dimensions, preserving user-visible size across 1×/2× monitors;
- `capture_scale_factor` records the source monitor scale solely to reconstruct the saved physical rectangle for monitor intersection;
- `maximized` is unchanged.

Capture keeps Tauri's physical outer position, converts outer size through the current window scale factor, and rounds only the logical size. Restore selects a monitor by intersecting the saved physical rectangle (`logical size × capture scale`) with physical monitor work areas. It then computes target physical bounds from the logical size and selected monitor scale, clamps the physical position in that work area, calls `Size::Logical` for size and `Position::Physical` for global placement, and maximizes last. No calculation mixes logical origins from different monitors.

Records without version 2 are legacy physical-pixel records and fail closed to the configured logical default `1280 × 820`, physically centered in the selected primary work area by the same pure geometry function. They are not guessed or reinterpreted. The next genuine user geometry change writes version 2, so invalid legacy state cannot become sticky.

Minimum `760 × 620` validates logical dimensions. Off-screen intersection/clamping stays physical and target-size-aware. Maximized state restores after normal geometry.

## Startup persistence gate

`WindowStatePersistenceGate` has explicit `PreArm`, `Restoring`, and `Ready` phases. `on_window_event` fail-closes geometry persistence while the managed gate is absent or `PreArm`, covering window-creation events before `.setup()`. Setup manages the gate before restore. The restore path computes one exact `AppliedWindowGeometry` (logical size, physical position, maximized) with the same pure function used by setters, arms `Restoring` before any `set_size`/`set_position`/`maximize` call, then applies it.

Before arming, restore captures the current geometry as `initial` and computes `expected`. `Restoring` is a finite value fence, not an acknowledgement wait: for each event it captures the complete current geometry through the same `capture_window_geometry` helper and rounding used by persistence. A geometry echo is suppressed when its logical size is either the initial or expected size and its physical position is either the initial or expected position; this admits intermediate `(expected size, initial position)` / `(initial size, expected position)` setter ordering and duplicate echoes without requiring either event to occur. The first non-maximized observation outside that finite cross-product is user intent, immediately moves to `Ready`, and persists. There is no matching-event prerequisite, so an unchanged restore/default followed by a user move cannot remain pinned.

The capture helper uses the live `window.scale_factor()` for both ordinary Resized/Moved observation and persistence; ScaleFactorChanged is evaluated only after querying the window's current outer geometry and live scale, so one conversion/rounding source owns fractional DPI. The restored `maximized` value is fenced independently from geometry. While live `window.is_maximized()` equals the restored value, maximize-generated geometry is suppressed according to the rules above (and all geometry is suppressed while both are true). The first event observing a different maximized value immediately moves to `Ready` and persists, regardless of whether size/position remain inside the initial/expected fence. Thus unmaximize back to the exact expected normal bounds still records `maximized: false`. CloseRequested/Destroyed persist only in `Ready`.

Default fallback does not call opaque `center()`: the pure geometry function computes the primary work-area center, so expected centering and applied centering use identical rounding. This is value/fence based, not time based: no sleep, debounce, page-load guess, or secure-backup-state coupling. A secure-backup gate may remain visible arbitrarily long without rewriting geometry; a genuine resize/move after startup echoes settle is persisted.

## Verify first

Pure deterministic tests precede runtime changes:

1. A legacy `1077 × 853` record is rejected; captured at 2× that physical geometry resolves below `760 × 620` logical.
2. Physical sizes representing the same logical size at 1× and 2× capture identically; restore on either scale computes the same logical size and correct target physical bounds.
3. Mixed-DPI placement uses physical global coordinates: a 2× secondary capture restored with a 1× primary still selects/clamps to the correct physical work area.
4. Version-2 JSON round-trips; unversioned legacy JSON loads as no restorable state.
5. Minimum validation is logical; invalid state selects exact default `1280 × 820` and deterministic primary centering, including odd-pixel rounding.
6. Off-screen/multi-monitor physical clamping and primary fallback remain deterministic.
7. Pre-arm events are suppressed; initial/expected values, duplicate echoes, and both intermediate setter-order combinations remain suppressed without waiting for an event acknowledgement.
8. When initial equals expected and setters emit no event, the first differing user resize/move immediately retires the gate and persists.
9. Fractional 1.25×/1.5× capture and ScaleFactorChanged observation use the same live-scale rounding and do not pin or falsely retire the fence.
10. Maximized restore events are suppressed while maximized; user unmaximize back to exact expected normal bounds independently retires and persists `maximized: false`.
11. Close/Destroyed during PreArm/Restoring does not persist; Ready close behavior remains.
12. Existing atomic-path, focus, close, and event-classification tests remain green.

## Implementation

Keep one private owner in `apps/desktop/src-tauri/src/window_state.rs` (moved without behavior changes by the #551 window-state seam). Keep conversion/selection/gate functions and their deterministic tests together there. Reuse the existing persistence path and atomic write. Do not add a crate/dependency, async task, timer, frontend command, or second state file.

## Gates

- `reviewer-flash-opencode-go` design verdict: v1–v3 `Not correct-to-merge` for mixed-DPI global coordinates, event-ack liveness, and unmaximize persistence; v4 `Correct-to-merge` after physical-position/logical-size schema and independent finite geometry/maximized fences resolved all blockers.
- `luna-implementer` at max thinking for verify-first implementation.
- RED exposed stale schema initializers/type mismatches; first GREEN review then found a maximized-echo overwrite defect. After the regression fix, full Tauri lib passed 158 tests / 1 ignored and `cargo fmt --check` exited 0.
- `reviewer-flash-opencode-go` full-diff v1 was `Not correct-to-merge` for fullscreen echo persistence; v2 reviewed the exact 1221-line patch and returned `Correct-to-merge` after the maximize-echo/unmaximize test and fence fix.
- Integrated full local matrix, CI, merge, issue evidence, and build-artifact cleanup in the shared PR.
