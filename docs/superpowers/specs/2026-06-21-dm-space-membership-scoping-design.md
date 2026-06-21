# DM Space-Membership Scoping (Model C1) — Design

- Date: 2026-06-21
- Status: Pending user review
- Branch: implementation should run on a feature branch off `main` (current
  `main` working tree carries the previous agent's mixed WIP; see §1).
- Canon impact: amends `docs/architecture/overview.md` and
  `docs/architecture/state-machine.md`, which currently state "DMs are global …
  regardless of active Space". This design intentionally changes that product
  rule, so canon is amended first (Canon-first redesign protocol).

## 1. Context & current state

### The original defect
A previous agent tried to scope DMs to Spaces by reusing Matrix
`parent_space_ids` plus an empty-list fallback in `compose_sidebar`. Because
DMs are account-level (`m.direct`) and normally carry no `m.space.child` /
`m.space.parent` edge, that logic could only ever produce two broken states:
0 DMs in a Space, or every DM piled into the one Space that has child rooms.
The tests were also rewritten to lock the broken behavior in.

### What this session already did (verified ground truth, not self-report)
The broken `compose_sidebar` DM logic was reverted and the DM contract restored
to "global", to get a clean, known-good base:

- `crates/koushi-state/src/sidebar.rs` reverted to HEAD: `global_dms` is
  `rooms.iter().filter(|room| room.is_dm)` (all DMs, unconditional).
- `apps/desktop/src/domain/desktopModel.ts` `composeSidebar` reverted: the
  `roomVisibleInActiveSpace` closure and fallback removed; `globalDms =
  rooms.filter((room) => room.is_dm)`.
- Tests updated to assert global DMs. Verified green:
  `cargo test -p koushi-state --test navigation_state` (22 passed),
  `npm --prefix apps/desktop run test -- --run src/domain/desktopModel.test.ts`
  (24 passed), `npm --prefix apps/desktop run typecheck` (clean).
- The Bug-4 (DM avatar projection) additions in those files were preserved.

So the current baseline is **DMs global** (the canon default). This design now
implements **model C1**, which deviates from that default; the "all DMs global
under a Space" tests added this session are replaced by C1 assertions.

### Render path (verified)
The real (Tauri) app's DM section is recomputed in TypeScript:
`Shell.tsx:360` → `roomListSections(...)` → `roomListSectionsFromSidebar` →
`composeSidebar(activeSpaceId, snapshot.spaces, snapshot.rooms)` →
`people = sidebar.global_dms`. The Rust `compose_sidebar` (via `dto.rs`) mirrors
the same logic and feeds headless QA, the browser build, and diagnostics. Both
compose paths must therefore agree on the DM rule.

### Out of scope for this design
The other three reported bugs (DM avatars, search-crawler UI, composer input
latency) are tracked separately and handled in the agreed order after this
work: avatars → crawler UI → composer. The previous agent's WIP for those
remains in the working tree untouched by this design.

## 2. Behavior specification

`compose_sidebar(active_space_id, spaces, rooms)` non-DM room scoping is
unchanged. The DM ("People"/"DMs") section becomes:

| Active selection | DM section contents |
|---|---|
| None (Home / account) | All DMs |
| Space `S` | DMs where at least one `dm_user_ids` member is a joined member of `S`'s space room |
| (consequence) | A DM whose partner(s) are in no space's membership appears only in Home |

Rules:
- **Group DMs** (`dm_user_ids.len() > 1`): visible in `S` if **any** partner is
  a member of `S`. A DM may therefore appear under multiple spaces.
- **Home always shows all DMs**, so no DM is ever unreachable.
- DM ordering, unread/highlight counts, and avatars are unchanged; only section
  membership changes.

## 3. Architecture

Chosen approach **B**: Rust core precomputes, per DM, the set of spaces it
belongs to, and ships that as a light field. Rejected alternatives:

| Approach | Why not |
|---|---|
| A — ship `SpaceSummary.member_user_ids` in the DTO and intersect in both compose paths | Large spaces put full member-ID lists into every snapshot (heavy, and member IDs in the DTO are needless exposure). |
| C — Rust fully filters and the frontend stops recomputing, reading `snapshot.sidebar.global_dms` directly | Cleanest ownership but requires changing `Shell.tsx` (a hot file) and the render path now; larger churn than warranted. Revisit later if the render path is consolidated. |

**B** keeps the DTO light, keeps the membership decision Rust-owned (React only
renders a Rust-provided fact), and leaves the existing render path intact.

## 4. Data model

- **New field `RoomSummary.dm_space_ids: Vec<String>`** — for a DM room, the
  space IDs whose membership includes at least one partner. Empty for non-DM
  rooms and for DMs with no matching space. This is the only new data crossing
  the Tauri DTO boundary.
- **Space member user IDs are NOT persisted** into `state`/DTO. They are
  obtained from the SDK and consumed transiently in `koushi-core` only to
  compute `dm_space_ids` during room-list normalization. (The SDK already
  enumerates room members to produce `joined_members` counts at
  `crates/koushi-sdk/src/lib.rs:4191`; this exposes the IDs for space rooms.)
- DTO mirrors to update in lock-step (per AGENTS.md "Core Batch A DTO Mirrors"):
  `apps/desktop/src-tauri/src/dto.rs` (`FrontendAppState`/`RoomSummary`),
  `apps/desktop/src/domain/types.ts`, `browserFakeApi.ts`, `tauriIpcMock.ts`,
  `appHarnessMain.tsx`, and the DTO serialization-contract test.

## 5. Data flow

1. **SDK** (`koushi-sdk`): when projecting the room list, include each space
   room's member user IDs alongside the existing space projection (used only as
   input to step 2; the exact carrier — a richer SDK-side space struct vs. a
   dedicated members map on the room-list payload — is an implementation-plan
   detail).
2. **Core** (`koushi-core`): during `RoomListUpdated` normalization, for each DM
   compute `dm_space_ids = { space_id | dm_user_ids ∩ members(space_id) ≠ ∅ }`
   and store it on the `RoomSummary`. Member lists are discarded after this.
3. **`compose_sidebar` (Rust) and `composeSidebar` (TS)**: the DM filter becomes
   `active_space_id.is_none() || dm.dm_space_ids.contains(active_space_id)`.
4. Membership changes arrive via the next sync → `RoomListUpdated` →
   `dm_space_ids` recomputed. There is no separate React-side recomputation of
   membership.

## 6. Canon amendment (do first)

Before code, amend:
- `docs/architecture/overview.md` (the "DMs are global … never duplicated under
  Spaces" statement, ~line 21).
- `docs/architecture/state-machine.md` (the DM/Space sidebar rules, ~line 175).

New rule text: "DMs are shown in full only in Home (no active Space). When a
Space is active, the DM section shows DMs where at least one DM counterpart is a
member of that Space's room. A DM with no matching Space appears only in Home.
DMs are never assigned to Spaces via `m.space.child`/`m.space.parent`; the
association is by counterpart space-room membership and is computed Rust-side as
`RoomSummary.dm_space_ids`."

## 7. Testing (TDD: failing test first, then implement)

- **Rust `compose_sidebar` unit** (`navigation_state.rs` / `sidebar` tests): the
  full matrix — Home shows all DMs; Space `S` shows only membership-matched DMs;
  unmatched DM appears only in Home; group DM appears under a Space if any
  partner matches; a DM matching two spaces appears under both.
- **Rust `dm_space_ids` computation unit** (core normalization): given spaces
  with member sets and DMs with partners, the derived `dm_space_ids` are
  correct; member lists are not retained in state.
- **TS `composeSidebar` unit** (`desktopModel.test.ts`): same matrix against the
  mirrored logic, reading `dm_space_ids`.
- **Headless QA guard** (`headless-core-qa`): after creating a space, joining a
  helper, and starting a DM with that helper, select the space and assert the DM
  is in the DM section under that space but a non-member DM is not; assert Home
  shows both. Private-data-free tokens only (e.g. `dm_space_scope=ok`); no room
  IDs, user IDs, or member lists in output. This closes the prior QA blind spot
  (the old `stress_space_scope=ok` forced `RoomListFilter::Rooms`, which excludes
  DMs, and asserted the already-correct projection, never the DM section).

## 8. Affected files

- `crates/koushi-sdk/src/lib.rs` — expose space-room member IDs.
- `crates/koushi-core/src/room.rs` (+ reducer/normalization) — compute
  `dm_space_ids`.
- `crates/koushi-state/src/state/room.rs` (`RoomSummary`),
  `crates/koushi-state/src/sidebar.rs` (DM filter),
  `crates/koushi-state` reducer/tests.
- `apps/desktop/src-tauri/src/dto.rs`, `apps/desktop/src/domain/types.ts`,
  `coreEvents.ts`/`coreEvents.generated.json` if a CoreEvent carries the field,
  `browserFakeApi.ts`, `tauriIpcMock.ts`, `appHarnessMain.tsx`,
  serialization-contract tests.
- `apps/desktop/src/domain/desktopModel.ts` + `desktopModel.test.ts` — DM filter
  by `dm_space_ids`.
- `crates/koushi-core/src/bin/headless-core-qa.rs` — QA guard scenario/tokens.
- `docs/architecture/overview.md`, `docs/architecture/state-machine.md` — canon.

## 9. Edge cases & non-goals

- **Membership not yet synced:** until a space's members are known,
  `dm_space_ids` may be empty and the DM shows only in Home; it appears under the
  space after sync populates membership. Acceptable; no loading spinner.
- **Self in members:** the current account is a member of its own spaces; matching
  is on the DM *counterpart* (`dm_user_ids`), which excludes self by construction,
  so this does not cause every DM to match every space.
- **Large spaces:** member lists are processed in Rust and never shipped; only the
  small per-DM `dm_space_ids` crosses the DTO.
- **Non-goals:** no UI for assigning DMs to spaces; no change to non-DM room
  scoping, DM ordering, avatars, or unread counts; no new sync of full member
  lists beyond what the SDK already enumerates.

## 10. Implementation order

1. Amend canon (§6).
2. Add `RoomSummary.dm_space_ids` + DTO mirrors (failing serialization/contract
   tests first).
3. Core `dm_space_ids` computation (failing unit first) + SDK space-member input.
4. `compose_sidebar` (Rust) DM filter (failing matrix unit first → implement).
5. `composeSidebar` (TS) DM filter (failing matrix unit first → implement) +
   typecheck.
6. Headless QA guard.
7. Full verification (Rust + TS suites, typecheck, headless local lane).
