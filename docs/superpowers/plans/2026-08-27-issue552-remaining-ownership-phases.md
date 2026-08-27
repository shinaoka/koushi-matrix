# Issue #552 Remaining Frontend Ownership Migration — Phased Execution Plan

Status: Phase 0 documentation implemented. No production implementation is authorized by this document alone.

Base: `origin/main` `28a3dfb927d950e8a6724a933cb92e0c51111a01`.

## Objective

Finish #552 incrementally: every durable product state machine or backend resource has one Rust owner unless a documented renderer-only reason requires frontend ownership. Keep React/Tauri shipping throughout, preserve current IPC shapes unless a separately reviewed contract migration requires a change, and leave a frontend-neutral Core boundary suitable for a future native Rust renderer.

This plan does not reopen #111, repeat #550 leak fixes, continue #551 move-only decomposition, or authorize a GPUI rewrite.

## Current baseline

Already shipped and not to be repeated:

- PR #674 published `docs/architecture/frontend-ownership-inventory.md` and identified Rust-owned, projection-cache, renderer-only, and test-mirror paths.
- PR #683 removed invite/mention query admission from the frontend mutation queue; Rust request/generation state plus `appStore` now owns convergence.
- `appStore` and `timelineStore` remain projection caches.
- Timeline DOM measurement, virtualization, scroll anchoring, focus, IME drafts, and visual overlays remain renderer-owned.
- #550 owns concrete detached-task/resource leak fixes; #551 owns behavior-preserving decomposition.

Phase 0 recon on this base confirms:

- The inventory's older active-design states are now refreshed against current main and merged issue/PR evidence.
- `DesktopApi` is declared in `backend/browserFakeApi.ts`, reversing production/fake dependency direction.
- Eight production frontend modules import `@tauri-apps/*` directly; three are under `domain/`.
- `TimelineView` still owns projection/repair acknowledgement retry counters and browser timers while Rust owns the actor-side acknowledgement terminal.
- App retains mutation sequencing for alias/caption writes and request refs for several command families; each requires separate equivalence proof before deletion.

## Phase rules

1. One ownership seam per PR. Structural adapter work and semantic ownership changes do not share a PR.
2. For a behavior change, add the deterministic RED check before implementation.
3. A frontend ref is deleted only when an existing or newly reviewed Rust authority covers stale input, replacement, failure, cancellation, and terminal settlement.
4. DOM evidence capture and renderer-local cleanup stay in React.
5. Browser Fake and harness code mirror the production contract; they are never cited as product authority.
6. Every phase updates the inventory row it changes and records design/final-diff reviewer verdicts.
7. Any Rust state/action/command/event or DTO change updates the complete snapshot/wire mirror list in `docs/agents/state-ownership.md`; reducer/state-machine changes amend `docs/architecture/state-machine.md` in the same PR.

## Phase 0 — Refresh the evidence baseline

**Deliverable:** one documentation-only PR.

1. Re-audit the existing inventory against the pinned base and current issue/PR state.
2. Mark designs now shipped, remove stale “active design” claims, and preserve explicit already-correct classifications.
3. Record the current direct-Tauri imports, neutral API location, remaining mutation queue users, acknowledgement retry owner, and request-ref families.
4. Map each #552 acceptance criterion to `complete`, `partial`, or `remaining` evidence.
5. Update this plan and the plan index if recon changes later phase boundaries.

**Exit:** every later PR names one exact inventory row and no shipped work is scheduled again.

**Phase 0 implementation record:** the inventory is pinned to the base above; records eight direct-Tauri production modules, the fake-owned `DesktopApi` declaration, alias/caption-only mutation queue, React/Tauri/Core acknowledgement boundary, and current App request-ref families; marks #559/#570/#582/#608/#659 and PR #683 as shipped; and preserves #552 as open with criterion-by-criterion complete/partial/remaining status. Production code, wire contracts, state machines, dependencies, and generated artifacts are unchanged.

## Phase 1 — Isolate the renderer transport without changing semantics

This phase improves dependency direction and future frontend portability. It does not count as a semantic migration by itself.

### Phase 1A — Neutral `DesktopApi` contract

**One PR.**

- Move the `DesktopApi` interface and contract-only supporting types from `backend/browserFakeApi.ts` to `backend/desktopApi.ts`.
- Make `client.ts` the Tauri implementation and `browserFakeApi.ts` a test implementation of that neutral contract.
- Move API implementation selection to the `appRuntime.ts` composition root, remove `client.ts`'s duplicate local `isTauriRuntime()` branch, and leave the separately scoped `tauriTimelineTransport.ts` adapter guard unchanged until its Phase 1B seam.
- Update imports only; do not rename IPC commands, alter DTOs, split the interface speculatively, or change behavior.

**Proof:** typecheck, focused client/fake/App tests, full Vitest, lint, build, and import-cycle check.

### Phase 1B — Platform ports at existing seams

**One PR per independently testable port family; do not bundle all families.**

Candidate order:

1. external links and media URL conversion/save;
2. native notification/attention operations;
3. window/dialog operations;
4. Core/state/menu event subscriptions.

For each family:

- define the smallest existing-operation port under `backend/`;
- place the Tauri implementation under `backend/tauri/`;
- preserve the fake/browser implementation;
- remove the corresponding direct Tauri import from `domain/`, hooks, or `App.tsx`;
- extend the frontend import guard so new `@tauri-apps/*` imports are admitted only in the approved adapter directory;
- when a family touches the shared hot file `App.tsx`, name the exact existing Tauri import being removed and change no unrelated App lifecycle in that PR.

**Exit:** `domain/**` has no Tauri imports; remaining direct imports are adapter-owned and statically enumerated. IPC names and serialized contracts are unchanged.

## Phase 2 — Projection acknowledgement reliability leaf

**Deliverable:** one semantic-owner PR, preceded by a reviewed design document.

### Recon and design gate

Trace the complete path:

```text
committed DOM evidence
  -> TimelineView acknowledgement intent
  -> Desktop transport/Tauri submit
  -> AppCommand
  -> TimelineManager/TimelineActor acknowledgement
  -> repair/projection terminal
```

Determine exactly which failures occur before command acceptance and which terminal is actor-owned. Do not assume that moving a timer to Rust improves reliability.

### Required RED evidence

Use fake clocks/barriers, never sleeps:

- transport rejects before command acceptance, then recovers;
- timeline key/actor generation changes while an acknowledgement is pending;
- component unmounts after DOM evidence but before transport acceptance;
- duplicate acknowledgement is idempotent;
- accepted acknowledgement settles without a renderer retry owner;
- shutdown/replacement cancels and awaits any Rust-owned retry task if one is introduced.

### Implementation decision

Choose the first option supported by the evidence:

1. **Delete retry policy:** if transport acceptance is the only required terminal and the existing command boundary can return it reliably.
2. **Adapter-owned retry:** if retries are purely local transport submission and do not need Core state.
3. **Rust-owned retry/settlement:** only if work must survive view disappearance and Core has a concrete retained owner with explicit cancel-and-await shutdown.

React keeps one-shot DOM evidence capture and generation data. Delete `projectionAcknowledgementRetryRef`, `repairAcknowledgementRetryRef`, and browser timers only after the selected owner is proven.

**Exit:** one documented acknowledgement owner, deterministic failure/replacement/teardown coverage, and no duplicate TS retry state.

## Phase 3 — Remove redundant App request fences by command family

**One family per PR.** Suggested order after fresh recon:

1. room settings;
2. diagnostics snapshot;
3. Space member open/invite/cancel/role and Space invite search, split further if their local lifetimes differ;
4. navigation/dialog-bound operations.

For each family:

1. identify Rust `RequestId`, demand generation, account/session generation, and `appStore` generation admission;
2. add delayed A/B/A, stale completion, account/selection replacement, failure, and retry tests;
3. distinguish semantic fencing from purely local dialog/selection lifetime;
4. delete only the ref/transition proven redundant;
5. keep local presentation fences when Rust cannot know whether a dialog still exists.

No generic TypeScript request manager and no generic Rust queue.

**Exit:** each migrated family has one Rust authority; retained refs are documented renderer-lifetime guards.

## Phase 4 — Resolve remaining text-mutation sequencing

**Separate design and PR per mutation family.**

### Alias mutations

Prove whether the Rust `Saving` admission and projected terminal can accept the latest user intent without frontend serialization. If not, add the smallest alias-specific Rust pending/latest rule before deleting the TS queue entry.

### Main/thread staged-upload captions

Prove ordering across A/B/A edits, target/account replacement, item removal, and stale snapshots. If Rust lacks revision admission, add a caption-specific revision/request fence; do not add a general-purpose mutation framework.

**Exit:** either the TS mutation queue is deleted, or every retained user has a documented renderer-only necessity. “Still convenient” is not sufficient.

## Phase 5 — Audit the frontend-neutral Core consumption boundary

**Deliverable:** one Rust integration-test PR unless all evidence already exists.

Verify without Tauri/WebView:

- runtime start and awaited shutdown;
- connection-scoped command submission;
- CoreEvent subscription and backpressure behavior;
- snapshot generation and resync after lag;
- resource cancellation on connection/runtime teardown.

Keep IPC-only DTOs in `src-tauri`. Move a presentation DTO into shared Rust only when a concrete non-Tauri consumer proves reuse; prefer existing `AppState`/Core types.

**Exit:** a test starts Core, sends a command, observes event/snapshot convergence, and shuts down cleanly with no Tauri type in the contract.

## Phase 6 — Epic completion audit

**Deliverable:** one documentation/closure PR or the final semantic PR’s documentation section.

1. Re-run the full ownership inventory against current main.
2. Map every #552 acceptance criterion to merged code, tests, reviewer verdict, and CI evidence.
3. Confirm all remaining frontend owners are renderer presentation, projection cache, adapter resource, or intentional test mirror.
4. Confirm migrated Rust async owners have explicit cancellation and awaited settlement where correctness requires it.
5. Confirm corresponding TS semantic state was removed.
6. Run full repository gates and close #552 only if no semantic owner remains merely “investigate”.

## Optional Phase 7 — Non-shipping GPUI vertical slice

This is separately approved follow-up work, not required to claim #552 semantic-ownership completion.

After Phase 5, a small `apps/gpui/` spike may consume Core directly for runtime/session state, room list, and a read-only timeline. Composer/send work starts only after Japanese/CJK IME, accessibility, virtualization, media, clipboard, notifications, and shutdown behavior have measurable gates. React/Tauri remains production until separately agreed parity exists.

## Verification matrix

Every PR runs its focused RED/GREEN check plus the relevant subset below; final semantic phases run the complete matrix:

```bash
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run lint
npm --prefix apps/desktop run test -- --run
npm --prefix apps/desktop run build
(cd apps/desktop && npx playwright test)
cargo fmt --all -- --check
cargo test --workspace
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib
cargo test -p koushi-core --features qa-bin --bin headless-core-qa
node scripts/check-sdk-submodule.mjs
node scripts/check-agents-docs.mjs
```

Run wire/DTO goldens, wasm, `cargo deny`, `cargo machete`, and local homeserver QA whenever the changed boundary reaches those surfaces. Read each command’s own exit status.

## Review gate

Selected independent reviewer: `reviewer-flash` (user-selected; read-only, independent model family).

Before each semantic implementation:

1. write the task-level design and RED proof;
2. obtain a recorded independent reviewer verdict from a different model family;
3. implement only after `Correct-to-merge` or after all findings are fixed and re-reviewed;
4. inspect the complete diff and obtain the same independent post-implementation verdict;
5. fix and re-review every finding before PR creation/merge.

## Design review record

- Round 1, `reviewer-flash`: initial blocking run exhausted its turn budget and issued no verdict; it did not satisfy the gate.
- Round 1 focused retry, `reviewer-flash`: `Correct-to-merge`. The reviewer verified the current ACK path through React, Tauri, TimelineManager and TimelineActor; confirmed the phase boundaries, renderer-only exclusions, non-semantic classification of adapter isolation, and optional GPUI scope; and found no Critical or Important issue.
- The four minor findings are applied: current direct-Tauri production imports are eight, state/wire mirror and state-machine obligations are explicit, `isTauriRuntime()` selection is assigned to `appRuntime.ts`, and each Phase 1B App hot-file edit must name the exact import removed.

## Explicit non-goals

- Removing Tauri or React.
- Turning `appStore`/`timelineStore` into cross-framework product authorities.
- Moving DOM, IME, focus, measurement, virtualization, animation, or drafts into Rust.
- Replacing `App.tsx` with another giant TypeScript controller.
- Generic request/retry/mutation frameworks without a proven second user.
- Combining multiple ownership seams, behavior changes, and UX changes in one PR.
- Building a shipping GPUI frontend inside the #552 closure path.
