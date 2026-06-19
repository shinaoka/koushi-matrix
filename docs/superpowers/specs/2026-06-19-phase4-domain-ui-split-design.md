# #87 Phase 4 — section the snapshot DTO into domain/ui (renderer in lockstep)

Date 2026-06-19. Issue #87 Phase 4 ("mobile seam, part 1"), sole checkbox: **"Split the snapshot
DTO into domain + ui sections; update the renderer in lockstep (version the IPC contract). Same
information, reorganized → behavior preserved, guarded by Phase 1 characterization."**

The mobile seam is the **serialized wire contract**, not the internal Rust representation. So this
phase nests the **snapshot DTO + the renderer only**; the Rust `koushi_state::AppState` struct STAYS
FLAT (nesting it would be ~682 field accesses + 96 `AppState{..}` construction sites — intricate
internal churn the contract does not require). codex reviewed and confirmed this DTO-only strategy
satisfies Phase 4.

## Exact field partition — the 39 `FrontendAppState` (DTO) fields
This ONE map is authoritative and must be identical across `dto.rs`, `types.ts`, the golden
flattener, and the residual grep.

**domain (30):** `session, auth, device_sessions, account_management,
account_management_capabilities, soft_logout_reauth, qr_login, settings, link_preview_settings,
locale_profile, typography_profile, profile, sync, sync_mode, spaces, rooms, invites,
room_notification_settings, room_interactions, directory, room_management, activity,
thread_attention, search, search_crawler, live_signals, e2ee_trust, local_encryption,
native_attention, cjk_text_policy`

**ui (9):** `navigation, room_list, timeline, thread, focused_context, files_view, threads_list,
basic_operation, errors`

(`upload_staging` and `media_gallery` are NOT top-level DTO fields — they live inside `timeline`
→ ui. `locale_profile`/`typography_profile` are Rust-owned resolved display profiles → domain;
`native_attention`/`e2ee_trust` are Rust-owned product/security state → domain.)

## Changes
1. **`apps/desktop/src-tauri/src/dto.rs`** (orchestrator-owned): `FrontendAppState` becomes
   `{ schema_version: u32, domain: FrontendDomainState, ui: FrontendUiState }`. Define the two
   sub-structs with the fields above (carrying the existing field TYPES, incl. the `Frontend*`
   wrapper types for session/sync/thread/search). `From<AppState>` keeps its existing
   profile/native_attention computation and maps the flat `state.*` into the nested sections; sets
   `schema_version: SNAPSHOT_SCHEMA_VERSION` (=2). Add `pub const SNAPSHOT_SCHEMA_VERSION: u32 = 2;`.
2. **`types.ts`** (orchestrator-owned): `AppState` becomes `{ schema_version: number; domain: {..};
   ui: {..} }` mirroring the partition.
3. **Renderer** (deepseek, typecheck oracle): rewrite every flat `snapshot.state.<field>` /
   `state.<field>` access to `.state.domain.<field>` or `.state.ui.<field>` per the map. ~868 sites;
   typecheck fails on each stale access until rewritten. (CoreEvent-backed stores — TimelineView
   rows, etc. — do NOT read `snapshot.state`, so they are unaffected.)
4. **Fakes/mock/harness** (orchestrator-owned hot files): nest the `browserFakeApi.ts`,
   `tauriIpcMock.ts`, `appHarnessMain.tsx` snapshot fixtures into `{ schema_version, domain, ui }`.
5. **Boundary version guard**: the App snapshot ingestion asserts `snapshot.state.schema_version ===
   2` so a stale flat snapshot / mismatched build fails loudly.

## Safeguards (from codex design review)
- **schema_version** (above) — fail loud on stale/mismatched snapshots.
- **Equivalence oracle** — capture the OLD flat golden (git HEAD). After regenerating the new nested
  golden, a verification script FLATTENS it via this partition map (drop `schema_version`, hoist
  `domain.*`+`ui.*` to top level) and compares value-for-value to the OLD golden. Accept the new
  golden ONLY if `flatten(new) == old` — proves a pure reorganization (catches changed defaults,
  omitted fields, altered projections).
- **Residual grep** — after the renderer rewrite, assert no old top-level field name is still read
  as flat `.state.<field>` anywhere in frontend source/fixtures/mocks/harness/serialized snapshots.
- **Hot-file ownership** — orchestrator owns dto.rs, types.ts, the fakes/mock/harness snapshots, the
  golden, and the DTO/golden contract tests; deepseek produces only the mechanical renderer
  access-path rewrites as draft patches, verified by typecheck + the equivalence oracle.

## Golden + contracts
Regenerate the full-AppState golden (`UPDATE_GOLDEN=1`) to the nested shape; the diff is a pure
re-nesting (values identical), proven by the equivalence oracle. Update the DTO
serialization-contract test to the nested shape + assert `schema_version`. CoreEvent wire-contract
artifact is unaffected.

## Also in this PR (combined per user request)
FINAL warnings cleanup (#15): the pre-existing `koushi-core` dead_code warnings + the 5 eslint
unused-disable directives in test files.

## Verification
`cargo build/test --workspace`, src-tauri build + tests (golden NEW shape + DTO contract +
CoreEvent unchanged), wasm, domain-deps, machete; `npm run typecheck` + `npm test` + `npm run build`
+ eslint (0 warnings after FINAL); Playwright `basic-operations` (--workers=1); the equivalence
oracle (`flatten(new golden) == old golden`); residual flat-field grep == 0. codex diff review.
