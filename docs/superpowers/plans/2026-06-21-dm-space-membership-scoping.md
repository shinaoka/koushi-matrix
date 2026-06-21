# DM Space-Membership Scoping (Model C1) Implementation Plan

> **For agentic workers:** Implement task-by-task. Each task ends with an independently verifiable deliverable. Code generation is delegated to Sonnet subagents; this plan fixes the contracts (files, interfaces, test assertions, verification commands), not the implementation bodies. Steps use `- [ ]` for tracking.

**Goal:** Show DMs in full only in Home; when a Space is active, show only DMs whose counterpart is a member of that Space's room (model C1).

**Architecture:** Rust core precomputes, per DM, the set of spaces whose membership includes a DM counterpart, exposed as the light field `RoomSummary.dm_space_ids`. Both `compose_sidebar` (Rust) and `composeSidebar` (TS) filter the DM section by it. Space member lists never cross the DTO boundary.

**Tech Stack:** Rust (`koushi-sdk`, `koushi-core`, `koushi-state`), Tauri DTO, TypeScript/React (`apps/desktop/src`), Vitest, `cargo test`, headless-core-qa.

**Spec:** `docs/superpowers/specs/2026-06-21-dm-space-membership-scoping-design.md`

## Global Constraints

- **DTO mirror lockstep:** any `RoomSummary` field change updates, in the same task: `crates/koushi-state/src/state/room.rs`, `apps/desktop/src-tauri/src/dto.rs`, `apps/desktop/src/domain/types.ts`, `browserFakeApi.ts`, `tauriIpcMock.ts`, `appHarnessMain.tsx`, and the dto serialization-contract test. Also every Rust/TS fixture constructing `RoomSummary`.
- **Rust-owned semantics:** React only reads `dm_space_ids`; it must not compute membership. No React-local DM/space state.
- **Private-data-free QA:** headless tokens only (e.g. `dm_space_scope=ok`); never print room IDs, user IDs, member lists, or raw SDK errors.
- **`dm_user_ids` excludes self** (verified: `koushi-sdk/src/lib.rs:4196`); membership matching is on counterparts only.
- **Commits:** do NOT commit per task. The `main` working tree holds the previous agent's mixed WIP; before any commit, create a feature branch and split the DM-C1 changes out — only when the user asks. Tasks end at "verify green."

---

### Task 1: Canon amendment (do first)

**Files:**
- Modify: `docs/architecture/overview.md` (the "DMs are global … never duplicated under Spaces" statement, ~line 21)
- Modify: `docs/architecture/state-machine.md` (DM/Space sidebar rules, ~line 175)

**Interfaces:** none (docs).

- [ ] **Step 1:** Replace the "DMs are global regardless of active Space" rule in both files with the C1 rule: "DMs are shown in full only in Home (no active Space). When a Space is active, the DM section shows DMs where at least one DM counterpart is a member of that Space's room (any counterpart for group DMs). A DM matching no Space appears only in Home. DMs are never assigned to Spaces via `m.space.child`/`m.space.parent`; association is by counterpart space-room membership, computed Rust-side as `RoomSummary.dm_space_ids`."
- [ ] **Step 2: Verify** the new text is present and the old absolute claim is gone.
  Run: `grep -n "dm_space_ids\|member of that Space" docs/architecture/overview.md docs/architecture/state-machine.md` → expect matches; `grep -n "remain visible regardless of active Space" docs/architecture/state-machine.md` → expect no stale absolute rule (or reworded).

---

### Task 2: Add `RoomSummary.dm_space_ids` field + DTO/type mirrors

**Files:**
- Modify: `crates/koushi-state/src/state/room.rs` (`RoomSummary`)
- Modify: `apps/desktop/src-tauri/src/dto.rs`, `apps/desktop/src/domain/types.ts`, `apps/desktop/src/backend/browserFakeApi.ts`, `apps/desktop/src/test/tauriIpcMock.ts`, `apps/desktop/src/test/appHarnessMain.tsx`
- Modify (fixtures): `crates/koushi-state/tests/navigation_state.rs` (`rooms()` and inline `RoomSummary` literals), `apps/desktop/src/domain/desktopModel.test.ts` (`roomSummary` helper), and any other `RoomSummary` constructors surfaced by the compiler/typecheck.
- Test: `apps/desktop/src-tauri/src/dto.rs` (or `lib.rs`) serialization-contract test.

**Interfaces:**
- Produces: `RoomSummary.dm_space_ids: Vec<String>` (Rust), `dm_space_ids: string[]` (TS). Default empty. Serializes as `dm_space_ids` (snake_case) in the DTO; `#[serde(default)]` on the Rust field.

- [ ] **Step 1: Write the failing test.** Extend the dto serialization-contract test so the expected `RoomSummary` JSON includes `"dm_space_ids": []` (and a non-empty case if the test builds one).
- [ ] **Step 2: Run, expect FAIL.** Run: `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` → expect FAIL (missing field / mismatch) or compile error.
- [ ] **Step 3: Implement (Sonnet).** Add the field everywhere in the Global-Constraints lockstep list with default empty; update all `RoomSummary` fixtures to include `dm_space_ids: Vec::new()` / `dm_space_ids: []`.
- [ ] **Step 4: Run, expect PASS.** Run: `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml`, `cargo test -p koushi-state`, `npm --prefix apps/desktop run typecheck` → all green. (At this point `dm_space_ids` is always empty; no behavior change yet.)

---

### Task 3: Core computation of `dm_space_ids` (pure function + unit)

**Files:**
- Create/Modify: `crates/koushi-core/src/room.rs` — a normalization step over the room list.
- Test: `crates/koushi-core/tests/` (new or existing room-normalization test file).

**Interfaces:**
- Produces: a function, signature exactly:
  `pub fn assign_dm_space_ids(rooms: &mut [koushi_state::RoomSummary], space_members: &std::collections::BTreeMap<String, std::collections::BTreeSet<String>>)`
  where `space_members` maps `space_id → set of member user IDs`. For each `room` with `is_dm`, set `room.dm_space_ids` to the sorted list of `space_id`s whose member set intersects `room.dm_user_ids`. Non-DM rooms get `dm_space_ids = vec![]`. Deterministic order (sorted).
- Consumes (Task 4): the SDK supplies `space_members`.

- [ ] **Step 1: Write the failing test.** Given two spaces `space-a` (members `{@alice}`) and `space-b` (members `{@bob}`), and DMs: `dm-alice` (`dm_user_ids=[@alice]`), `dm-bob` (`dm_user_ids=[@bob]`), `dm-carol` (`dm_user_ids=[@carol]`), `dm-group` (`dm_user_ids=[@alice,@bob]`). Assert after `assign_dm_space_ids`: `dm-alice.dm_space_ids==["space-a"]`, `dm-bob==["space-b"]`, `dm-carol==[]`, `dm-group==["space-a","space-b"]`. Also assert a non-DM room stays `[]`.
- [ ] **Step 2: Run, expect FAIL.** Run: `cargo test -p koushi-core assign_dm_space_ids` → FAIL (function missing).
- [ ] **Step 3: Implement (Sonnet).** Implement `assign_dm_space_ids` as specified (intersection, sorted, DM-only).
- [ ] **Step 4: Run, expect PASS.** Run: `cargo test -p koushi-core assign_dm_space_ids` → PASS.

---

### Task 4: SDK supplies space-room member IDs into core normalization

**Files:**
- Modify: `crates/koushi-sdk/src/lib.rs` (room-list snapshot building, ~`matrix_room_list_*` near line 4143–4226) — for space rooms, include the room's member user IDs (`active_user_ids`) in the snapshot.
- Modify: `crates/koushi-core/src/room.rs` — build the `space_members` map from the SDK snapshot and call `assign_dm_space_ids` during normalization so emitted `RoomSummary`s carry `dm_space_ids`.

**Interfaces:**
- Consumes: Task 3 `assign_dm_space_ids`.
- Produces: normalized `RoomSummary`s (in the `RoomListUpdated` path) already carrying correct `dm_space_ids`. `koushi-state` stores them unchanged; member lists are NOT added to `state`/DTO.

- [ ] **Step 1: Write the failing test.** In `koushi-core` (or `koushi-sdk` test infra), drive the normalization path with a synthetic snapshot: one space room with a member, one DM with that member as counterpart, one DM with a non-member. Assert the resulting DM `RoomSummary.dm_space_ids` reflect membership (member-DM → `[space]`, other → `[]`). Use the existing SDK/core test fixtures pattern.
- [ ] **Step 2: Run, expect FAIL.** Run the focused test → FAIL (members not propagated / `dm_space_ids` empty).
- [ ] **Step 3: Implement (Sonnet).** Carry space-room member IDs from the SDK snapshot into core normalization; build `space_members` and call `assign_dm_space_ids`. Keep member lists out of `state`/DTO (transient in normalization only).
- [ ] **Step 4: Run, expect PASS.** Run the focused test + `cargo test -p koushi-sdk` + `cargo test -p koushi-core` → green.

---

### Task 5: Rust `compose_sidebar` DM filter by `dm_space_ids`

**Files:**
- Modify: `crates/koushi-state/src/sidebar.rs` (`global_dms` computation)
- Test: `crates/koushi-state/tests/navigation_state.rs` (replace the global-DM tests added this session)

**Interfaces:**
- Consumes: `RoomSummary.dm_space_ids`.
- DM filter becomes: `room.is_dm && (active_space_id.is_none() || room.dm_space_ids.iter().any(|s| Some(s.as_str()) == active_space_id))`. No fallback.

- [ ] **Step 1: Write/replace failing tests (the C1 matrix).** Update fixtures so `rooms()` DMs carry `dm_space_ids` (e.g. `dm-a.dm_space_ids=["space-a"]`). Rewrite:
  - `compose_sidebar(None, …)` → DM section = all DMs.
  - `compose_sidebar(Some("space-a"), …)` with `dm-a` (`dm_space_ids=["space-a"]`) and `dm-outside` (`dm_space_ids=[]`) → DM section = `["dm-a"]`, `dm-outside` excluded.
  - a DM with `dm_space_ids=["space-a","space-b"]` appears under both `space-a` and `space-b`.
  Rename the session's `active_space_sidebar_keeps_all_dms_global` and `selecting_space_filters_rooms_and_keeps_dms_global` to C1-accurate names asserting the above.
- [ ] **Step 2: Run, expect FAIL.** Run: `cargo test -p koushi-state --test navigation_state` → FAIL (current `global_dms` ignores `dm_space_ids`).
- [ ] **Step 3: Implement (Sonnet).** Apply the DM filter above in `sidebar.rs`.
- [ ] **Step 4: Run, expect PASS.** Run: `cargo test -p koushi-state` → green.

---

### Task 6: TS `composeSidebar` DM filter by `dm_space_ids`

**Files:**
- Modify: `apps/desktop/src/domain/desktopModel.ts` (`composeSidebar`)
- Test: `apps/desktop/src/domain/desktopModel.test.ts` (replace the global-DM tests added this session)

**Interfaces:**
- Consumes: `RoomSummary.dm_space_ids`.
- DM filter becomes: `rooms.filter((room) => room.is_dm && (activeSpaceId === null || room.dm_space_ids.includes(activeSpaceId)))`.

- [ ] **Step 1: Write/replace failing tests (the C1 matrix, mirroring Task 5).** Update the `roomSummary` helper / fixtures so DMs carry `dm_space_ids`. Assert: Home → all DMs; `space-a` → only DMs whose `dm_space_ids` includes `space-a`; a DM in two spaces appears under both. Replace `"active space keeps all DMs global"` with a C1 assertion.
- [ ] **Step 2: Run, expect FAIL.** Run: `npm --prefix apps/desktop run test -- --run src/domain/desktopModel.test.ts` → FAIL.
- [ ] **Step 3: Implement (Sonnet).** Apply the DM filter in `composeSidebar`.
- [ ] **Step 4: Run, expect PASS.** Run: `npm --prefix apps/desktop run test -- --run src/domain/desktopModel.test.ts` + `npm --prefix apps/desktop run typecheck` → green.

---

### Task 7: Headless QA guard (close the prior blind spot)

**Files:**
- Modify: `crates/koushi-core/src/bin/headless-core-qa.rs` (extend the `invites_dm` or a focused space-scope scenario)

**Interfaces:** private-data-free token `dm_space_scope=ok`.

- [ ] **Step 1: Add the assertion.** After creating a space, joining a helper, and starting a DM with that helper: bounded `SyncOnce` until membership/`dm_space_ids` settle; select the space and assert the DM appears in the DM section under that space; assert a control DM with a non-member counterpart does NOT; assert Home shows both. Emit `dm_space_scope=ok`. No room/user IDs or member lists in output.
- [ ] **Step 2: Run the local lane.** Run: `PATH=/tmp/koushi-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:headless-local -- --server=conduit --scenario=invites_dm --core --core-backend=probed --timeout-ms=240000` → expect `dm_space_scope=ok` (and existing tokens still green). Adjust the scenario name if a dedicated scenario is added.

---

### Task 8: Full verification

- [ ] **Step 1:** `cargo test -p koushi-state -p koushi-core -p koushi-sdk` → green.
- [ ] **Step 2:** `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` (DTO contract) → green.
- [ ] **Step 3:** `npm --prefix apps/desktop run test -- --run` (full vitest) and `npm --prefix apps/desktop run typecheck` → green.
- [ ] **Step 4:** the Task 7 headless lane → `dm_space_scope=ok`.
- [ ] **Step 5:** Report results; do NOT commit. Surface to the user: feature-branch + split-out-of-WIP plan for committing, and that the real dmg must be rebuilt to see C1 live.

## Self-Review

- **Spec coverage:** §2 behavior → T5/T6 (filter) + T3 (membership compute) + T1 (canon); §4 data model → T2; §5 data flow → T3/T4; §6 canon → T1; §7 testing → T3/T5/T6 units + T7 QA; §8 files → covered across tasks. No gaps.
- **Placeholders:** none; the SDK carrier detail (T4) is bounded to `active_user_ids` for space rooms.
- **Type consistency:** `dm_space_ids: Vec<String>` / `string[]` used identically in T2/T3/T5/T6; `assign_dm_space_ids` signature fixed in T3 and consumed in T4; filter predicate identical in T5 (Rust) and T6 (TS).
