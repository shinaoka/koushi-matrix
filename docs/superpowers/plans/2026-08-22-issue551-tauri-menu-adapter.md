# Issue #551 Tauri Menu Adapter

## Scope

Move the complete desktop-menu construction/action seam from `apps/desktop/src-tauri/src/lib.rs` into private `src/desktop_menu.rs`. This is a move-only ownership split after #656; Tauri bootstrap and the exhaustive command registry remain in `lib.rs`.

Baseline `d400d5d0`: `lib.rs` 4,187 lines / 169,020 bytes / SHA-256 `b58f148e901fa96bd56bf7358dba8af6d2ec7d44556ee0921d6d5fdf7f7c393b`.

## Exact ownership

Move these declarations together:

- `MENU_EVENT_NAME` and all five `MENU_ID_*` constants;
- `DesktopMenuItem`, test-only `DesktopStandardMenuItem`;
- `desktop_menu_items`, test-only `desktop_standard_menu_items`;
- `desktop_menu_action_id`;
- `build_desktop_menu` and private `menu_item`;
- macOS-only `toggle_main_window_fullscreen`.

Bodies, attrs, cfgs, labels, IDs, accelerators, action tokens, order and Tauri builder calls remain exact. Add only `pub(super)` visibility required by the parent to the event constant and parent-called functions/constants; preserve existing `pub(crate)` test surfaces. `menu_item` remains leaf-private.

The leaf owns its exact Tauri menu imports (`Emitter` is not needed; `Manager`, `MenuBuilder`, `MenuItemBuilder`, `SubmenuBuilder` are). Parent uses one unconditional group for `MENU_EVENT_NAME`, `build_desktop_menu`, and `desktop_menu_action_id`; one macOS-gated group for `MENU_ID_TOGGLE_FULLSCREEN` and `toggle_main_window_fullscreen`; and one test-gated group for `desktop_menu_items` and `desktop_standard_menu_items`. No re-export façade or barrel.

## Invariants

- `run()` setup/menu callback behavior remains byte-equivalent at call sites.
- macOS fullscreen handling remains before action emission.
- command registry183, managed runtime state, forwarder, window geometry and OIDC code are unchanged.
- no menu listener/task/state/resource owner is added.
- static test expectations and platform cfg matrix remain unchanged.

## Verification

AST/item verifier from immutable baseline:

- moved declarations/functions/constants exact by kind/name and order;
- parent definitions0, one private module declaration, the three exact unconditional/macOS/test import groups, unchanged parent calls;
- leaf expected visibility/export surface only, no glob/default export;
- command registry183 and `serialize_core_event` exact;
- all non-menu production declarations, class/state fields and public wire names exact.

Run Tauri focused menu tests baseline/post x3, full Tauri/Core/frontend/QA/policy matrix, design/full-diff review, CI7/7 and #551 evidence.

## Implementation evidence

- Tauri menu baseline2/2 x3; post-move2/2 x3; full Tauri150/1 ignored plus keyring5.
- Exactness verifier: functions6/6, constants6/6, structs2/2 exact; parent definitions0; module/import cfg groups exact; registry183 exact; no glob.
- Metrics: parent4,187→4,029 lines; leaf166; combined4,195 (+8 lines / +386 bytes from explicit module/import visibility).
- Post-implementation full-diff review: `reviewer-flash` `Correct-to-merge`; no findings. Full matrix pending.

## Gates

- `reviewer-flash` design verdict: `Correct-to-implement`; cfg-gated parent import finding incorporated.
- move-only exactness verification.
- `reviewer-flash` full-diff `Correct-to-merge`.
- latest-main merge and cleanup.
