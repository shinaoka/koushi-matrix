# State Transport Architecture — Incremental Slice Deltas + Selector Store (+ Channels)

- Date: 2026-06-21
- Status: Pending user review
- Goal/depth: FULL (Rust core emits incremental slice deltas; frontend moves to a Zustand selector-subscribed store; high-frequency streams via Tauri Channel). Behavior-preserving, phased.
- Branch: implement on a feature branch off `main`.
- Canon impact: amends `docs/architecture/overview.md`, `docs/architecture/state-machine.md`, and `AGENTS.md` (transport + frontend state model). Canon-first: amend before/with the change.
- Delegation: this spec is the **detailed design**. The implementing agent turns it into a plan (writing-plans) and executes via subagent-driven-development, phase by phase. Brainstorm only the Open Questions (§13) with the owner; do not re-decide the architecture.

## 1. Context & current state (verified ground truth)

The frontend consumes Rust state by **replacing the entire snapshot on every backend state change**:

- `AppActor::publish_snapshot()` emits `CoreEvent::StateChanged(self.state.clone())` — a **full state clone** — for *every* state-changing batch: command batches (`crates/koushi-core/src/runtime.rs:585`), background async actions (`runtime.rs:597-624`), emission at `runtime.rs:1929-1933`. Doc: "StateChanged per processed command batch" (`runtime.rs:8`).
- Tauri forwards it as the `koushi-desktop://state` webview event.
- `App` listens, debounces `STATE_EVENT_REFRESH_DEBOUNCE_MS = 250` (`apps/desktop/src/App.tsx:173`, `:1453`), then `refresh()` → `setSnapshot(await api.getSnapshot())` (`App.tsx:1552`) — replacing the whole `DesktopSnapshot` and re-rendering the tree. The snapshot is prop-drilled; `TimelinePane` takes the whole `snapshot` (`apps/desktop/src/components/panes.tsx:522`) and rebuilds `forwardDestinationsFromSnapshot`/`mentionCandidatesFromSnapshot` per render (`panes.tsx:802,853`).

High-frequency producers (search-crawler progress; live sync: messages, typing, receipts, presence, room-list) make this churn — the **root cause** behind #107 (composer latency residual) and general background jank.

**Precedent already in the codebase:** the timeline already uses **incremental CoreEvent diffs** (`InitialItems`/`ItemsUpdated`) into a dedicated store, plus **virtualization** (`apps/desktop/src/components/TimelineView.tsx:1335`, per-row `TimelineItemRow` at `:2058`). This design generalizes that philosophy to the rest of AppState.

**External reference (verified, incl. the vendored checkout):** `matrix-sdk-ui` Timeline `subscribe()` returns `(initial Vector, Stream<Vec<VectorDiff>>)` — batched incremental ops over a persistent immutable vector, full `Reset` only on subscriber lag (`matrix-rust-sdk/crates/matrix-sdk-ui/src/timeline/mod.rs:300-303`). This is the canonical model we mirror.

## 2. Goals & non-goals

Goals:
- Eliminate whole-tree re-renders caused by full-snapshot replacement on background state changes.
- Reduce IPC payload/serialization cost during high-frequency updates.
- Keep Rust as the owner of product state; the frontend store is a **projection cache** of Rust state, not a React-owned source of truth.
- Behavior-preserving; each phase ships independently and is gated by tests + QA lanes.

Non-goals:
- No product-behavior changes (sidebar/DM scoping, timeline content, etc.).
- No full VectorDiff rewrite of every AppState field (only hot lists, optionally — DA).
- No adoption of a new framework beyond a state library (Zustand).

## 3. Target architecture (three layers)

1. **Rust core — slice deltas.** The reducer/runtime tracks which top-level `AppState` slices changed in a batch and emits `CoreEvent::StateDelta { generation, changed }`, where `changed` carries only the changed slices' new values. `StateChanged(full)` is demoted to **initial load + reset fallback** only.
2. **Frontend — Zustand selector store.** `appStore.ts` mirrors `AppState` slices. The Tauri handler applies deltas (merging changed slices, preserving references for unchanged ones). Components subscribe with `useAppStore(s => s.slice)` + `useShallow`; derived arrays (mention/forward candidates) become **memoized selectors**. The store is a projection of Rust state — React never mutates product state in it; it still dispatches typed commands.
3. **Tauri transport — Channel for high-frequency.** Low-frequency deltas use the event system; high-frequency streams (crawler progress; possibly typing/receipts) move to a **Tauri Channel** (ordered, fast), measurement-gated.

## 4. Design decisions

- **DA — Delta granularity = per-slice replacement (primary), intra-slice VectorDiff for hot lists only.** A `StateDelta` carries whole changed top-level slices (e.g. `sidebar`, `search_crawler`, `live_signals`). Large/high-churn lists (timeline already; `rooms` next if measurement warrants) may additionally use VectorDiff-style intra-slice diffs. Rationale: per-slice replacement removes whole-snapshot transport + whole-tree re-render with far less complexity than VectorDiff-everywhere; intra-slice diffs are a targeted optimization, not a prerequisite.
- **DB — Phasing (each phase independently shippable, behavior-preserving):**
  - **Phase 0 — measure.** Instrument emit rate / payload size (Rust) and React render counts (frontend) to baseline and validate each phase. Decides throttle-vs-Channel.
  - **Phase 1 — frontend store, no transport change.** Introduce the Zustand store fed by the EXISTING full snapshot. The apply step **compares each incoming slice to the stored slice by value (shallow/structural equality) and replaces only changed slices** — because a freshly-deserialized snapshot has new references for *every* slice, unchanged slices must explicitly keep their *prior* store reference. Migrate components from prop-drilled `snapshot` to selectors + memoized derived selectors; add `React.memo` to hot subtrees (timeline/composer). This alone largely resolves #107 (selector subscribers re-render only when their slice's value actually changed).
  - **Phase 2 — core deltas.** Core emits `StateDelta` (changed slices); frontend applies to the store. Full snapshot becomes initial/reset only.
  - **Phase 3 — Channel for high-frequency.** Move crawler progress (and similar) to a Tauri Channel or dedicated throttle, gated by Phase 0 measurement.
  - **Phase 4 — intra-slice list diffs (optional).** VectorDiff for `rooms` etc. if per-slice replacement still re-renders large lists noticeably.
- **DC — Channel scope = high-frequency only, measurement-gated.** Keep low-frequency deltas on the event system. Adopt a Channel only where Phase 0 shows the event path is the bottleneck. Heed the Channel memory-leak issue (tauri #13133); the raw-event-payload optimization (tauri PR #14269) is unmerged, so use Channels for the raw-by-ID pattern if needed.

## 5. Data model & flow

- **`CoreEvent::StateDelta { generation: u64, changed: ChangedSlices }`** where `ChangedSlices` is a struct of `Option<Slice>` per top-level `AppState` slice (only `Some` for changed slices). `generation` is a monotonically increasing per-session counter.
- **Initial load / reconnect / reset:** full snapshot (`StateChanged` or a `StateReset { generation, state }`).
- **Ordering & gap detection:** deltas apply in `generation` order. If the frontend sees a gap (received generation ≠ last + 1), it requests a full snapshot (reset fallback) — mirrors the SDK's Reset-on-lag.
- **DTO mirrors (lockstep):** the delta/changed-slices shape and `CoreEvent::StateDelta` must update `apps/desktop/src-tauri/src/dto.rs`, `apps/desktop/src/domain/types.ts`, `coreEvents.ts` + `coreEvents.generated.json`, browser fakes, IPC mock, app harness snapshots, and the serialization-contract tests together.

## 6. Components & boundaries

- **Rust:** a `state_delta` module in `koushi-core` that, given the pre/post `AppState` of a batch (or reducer-tracked dirty flags), produces `ChangedSlices`; `CoreEvent::StateDelta` variant; `publish_snapshot` becomes `publish_delta` (+ keep full publish for initial/reset). Keep diffing cheap (slice-level `PartialEq`/dirty-tracking, not deep structural diff).
- **Frontend:** `apps/desktop/src/domain/appStore.ts` — Zustand store with one entry per AppState slice; `applyDelta(changed)` (applies the slices present in the delta) and `applySnapshot(full)` (value-compares every slice, replaces only those that changed, keeps the prior reference for unchanged slices); both leave unchanged slices reference-stable so selectors don't re-render; selector hooks; memoized derived selectors (mention/forward candidates). `App.tsx` event/Channel handler calls these. Components migrate from `snapshot.state.X` to `useAppStore`.
- **Tauri:** event for deltas (Phase 2); a Channel for high-frequency (Phase 3).

## 7. Correctness & error handling

- **Behavior-preserving invariant:** applying the sequence of deltas yields a store state identical to the equivalent full snapshot (test §9).
- **Gap → reset:** missed generation → request + apply full snapshot; never silently diverge.
- **Initial & reconnect:** always a full snapshot first.
- **Reducer dirty-tracking must be complete:** every reducer path that mutates a slice must mark it changed, or the delta will omit a real change. This is the highest-risk correctness area — cover with the equivalence test and a dirty-tracking audit.

## 8. Phased migration (ship order)

Phase 0 → 1 → 2 → 3 → (4 optional). Each phase: green existing suites + its own regression + (where applicable) the Linux GUI / headless QA lane, behavior unchanged. Phase 1 is the de-risking step and delivers the #107 win without touching the transport. Phases 2–4 reduce transport cost and large-list churn.

## 9. Testing strategy (TDD per phase)

- **Render-isolation regression (generalizes #107):** emit synthetic delta/snapshot for an UNRELATED slice while typing → assert the composer/timeline subtree does not re-render (render counter) and `ui_long_frames`/`ui_frame_max_ms` (`apps/desktop/src/domain/uiLatency.ts`) stay under threshold.
- **Delta = snapshot equivalence (Rust + TS):** for a sequence of mutations, the delta-applied store equals the full-snapshot store (property-style).
- **Gap → reset fallback:** a skipped generation triggers a full-snapshot request and recovery.
- **DTO contract:** `StateDelta`/changed-slices wire shape; keep `core_event_wire_format_matches_checked_in_contract_artifact` green (note: that test is currently red from unrelated WIP — issue #109 — and must be addressed before Phase 2's contract changes land cleanly).
- Existing Rust + vitest + Playwright suites stay green per phase.

## 10. Canon amendments (do first, per phase as shapes settle)

- `docs/architecture/overview.md` / `state-machine.md`: the transport is **incremental slice deltas with full-snapshot reset fallback**; the frontend holds a **selector-subscribed projection store** (Rust still owns product state; the store is a cache fed by deltas, not a React-owned source of truth); high-frequency streams may use a Tauri Channel.
- `AGENTS.md`: add a "State Transport" note (delta/reset contract, DTO-mirror lockstep for delta shapes, the projection-store rule, Channel usage + caveats).

## 11. Risks & research caveats

- **Per-slice replacement still re-renders that slice's subtree.** Mitigate hot lists with intra-slice diffs (Phase 4) and `React.memo` on row components. Virtualization alone does NOT prevent whole-tree re-renders on a top-level replace (verified, 2-1) — selectors + memoization are required.
- **Reducer dirty-tracking completeness** is the main correctness risk (see §7); the equivalence test is the guard.
- **Tauri "high-frequency emit crashes the app" evidence is weak/old** (Win10/WebView2/Tauri 1.5.2, pre-2.0 IPC rewrite) — treat as a scaling warning; measure (Phase 0) before assuming a Channel is required.
- **Channel memory-leak (tauri #13133)**; raw-event-payload optimization (tauri PR #14269) unmerged — use Channels for the raw-by-ID pattern today.
- **Zustand transient updates** (`store.subscribe`+`useRef`) are an escape hatch ("not recommended for Reactish apps" under concurrent rendering) — scope strictly to non-rendering counters (e.g. crawl progress), not general slice subscriptions. "Jotai removes the need for memoization" was refuted (1-2) — not a basis for choosing Jotai.

## 12. References

- matrix-sdk-ui Timeline `subscribe()` + `VectorDiff` (vendored: `matrix-rust-sdk/crates/matrix-sdk-ui/src/timeline/mod.rs:300`): https://docs.rs/matrix-sdk-ui/latest/matrix_sdk_ui/timeline/struct.Timeline.html
- Tauri calling-frontend (event vs Channel): https://v2.tauri.app/develop/calling-frontend/ ; Channel: https://docs.rs/tauri/latest/tauri/ipc/struct.Channel.html
- Zustand (selectors / `useShallow` / transient): https://github.com/pmndrs/zustand
- Spacedrive (Tauri2+React19, per-op typed queries): https://github.com/spacedriveapp/spacedrive
- tauri-plugin-matrix-svelte — **negative example** (re-serializes whole store per update): https://github.com/IT-ess/tauri-plugin-matrix-svelte

## 13. Open questions for the implementing agent (brainstorm with owner)

- Can `AppState` be made cheaply slice-diffable (reducer dirty-flags vs pre/post `PartialEq` per slice), or does it need restructuring into clearly separable slices first?
- Actual emit rate + payload size of crawler progress and live sync (Phase 0) — decides throttle vs Channel (DC).
- Component migration order in Phase 1 (which subtrees move to selectors first — timeline/composer/sidebar) to bound risk.

## 14. Acceptance

- Background state changes (crawler/sync) no longer re-render unrelated subtrees; typing stays immediate under high-frequency traffic (resolves #107's residual).
- Full-snapshot replacement removed from the normal update path (initial/reset only).
- Behavior unchanged across all phases; canon updated. Each phase independently shippable and green.
