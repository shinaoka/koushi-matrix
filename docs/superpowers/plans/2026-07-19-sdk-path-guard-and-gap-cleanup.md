# SDK Path Guard and Gap Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prevent the desktop from compiling an SDK other than the checked-in submodule, freeze the repaired anchored-gap behavior, and consolidate duplicated unprojected-gap selection without changing recovery semantics.

**Architecture:** The superproject gitlink is the only SDK revision pin and all five workspace SDK crates resolve by path. A focused Node guard enforces that build-input contract. Recovery behavior is fixed first at the SDK and real proxy/Core boundaries; only then are three Core fallback variants represented by one typed `Unprojected` selection with an explicit reason.

**Tech Stack:** Node.js built-in test runner, Cargo/Rust, Matrix Rust SDK event cache, Koushi Core actor tests and headless proxy QA.

## Global Constraints

- The five guarded Matrix SDK dependencies must use exact paths below `vendor/matrix-rust-sdk`.
- Root Git URL and `rev` declarations for guarded SDK crates are forbidden.
- The submodule may be locally dirty during development, but it must be initialized at the superproject gitlink checkout.
- Write and observe each focused regression test fail before changing its production implementation.
- Complete coherent implementation before running the long homeserver QA once.
- Do not redesign the causal projection/React render-ACK protocol in this batch.

---

### Task 1: Replace the obsolete revision guard with a path contract

**Files:**
- Modify: `scripts/sdk-submodule-guard.test.mjs`
- Modify: `scripts/lib/sdk-submodule-status.mjs`
- Modify: `scripts/check-sdk-submodule.mjs`

**Interfaces:**
- Produces: `assertSdkWorkspaceUsesSubmodulePaths({ repoRoot, manifestPath? })`
- Preserves: `assertSdkSubmoduleSynced({ repoRoot, fixturePath?, manifestPath? })`

- [ ] **Step 1: Write failing manifest-contract tests**

Add fixture helpers and import the new assertion:

```javascript
import { mkdtempSync, writeFileSync } from "node:fs";
import {
  assertSdkSubmoduleSynced,
  assertSdkWorkspaceUsesSubmodulePaths,
  parseSubmoduleStatus,
} from "./lib/sdk-submodule-status.mjs";

const VALID_SDK_DEPENDENCIES = `
[workspace.dependencies]
matrix-sdk = { path = "vendor/matrix-rust-sdk/crates/matrix-sdk" }
matrix-sdk-base = { path = "vendor/matrix-rust-sdk/crates/matrix-sdk-base" }
matrix-sdk-search = { path = "vendor/matrix-rust-sdk/crates/matrix-sdk-search" }
matrix-sdk-test = { path = "vendor/matrix-rust-sdk/testing/matrix-sdk-test" }
matrix-sdk-ui = { path = "vendor/matrix-rust-sdk/crates/matrix-sdk-ui" }
`;
```

Assert that the valid fixture and repository manifest pass. Assert that Git/rev, wrong path, missing declaration, duplicate declaration, and mixed path/Git fixtures throw a private-data-free error containing `must resolve from vendor/matrix-rust-sdk`.

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```bash
node --test scripts/sdk-submodule-guard.test.mjs
```

Expected: FAIL because `assertSdkWorkspaceUsesSubmodulePaths` is not exported.

- [ ] **Step 3: Implement strict path parsing**

Replace `readPinnedSdkRevision` and the SDK URL constant with an exact map:

```javascript
const SDK_DEPENDENCY_PATHS = new Map([
  ["matrix-sdk", "vendor/matrix-rust-sdk/crates/matrix-sdk"],
  ["matrix-sdk-base", "vendor/matrix-rust-sdk/crates/matrix-sdk-base"],
  ["matrix-sdk-search", "vendor/matrix-rust-sdk/crates/matrix-sdk-search"],
  ["matrix-sdk-test", "vendor/matrix-rust-sdk/testing/matrix-sdk-test"],
  ["matrix-sdk-ui", "vendor/matrix-rust-sdk/crates/matrix-sdk-ui"],
]);

export function assertSdkWorkspaceUsesSubmodulePaths({ repoRoot, manifestPath } = {}) {
  const source = readFileSync(manifestPath ?? join(repoRoot, "Cargo.toml"), "utf8");
  for (const [name, expectedPath] of SDK_DEPENDENCY_PATHS) {
    const declarations = [...source.matchAll(new RegExp(`^${name}\\s*=\\s*\\{([^}]*)\\}`, "gm"))];
    const body = declarations[0]?.[1] ?? "";
    const path = /(?:^|,)\s*path\s*=\s*"([^"]+)"/.exec(body)?.[1];
    if (declarations.length !== 1 || path !== expectedPath || /(?:^|,)\s*(?:git|rev)\s*=/.test(body)) {
      throw new Error("Matrix SDK workspace dependencies must resolve from vendor/matrix-rust-sdk paths");
    }
  }
}
```

Call this assertion before reading submodule status. Remove `expectedRevision` and revision comparison; a leading-space `git submodule status` already proves checkout equals the superproject gitlink.

- [ ] **Step 4: Remove the obsolete CLI revision option**

Delete `--expected-rev` parsing and update CLI tests to use a manifest fixture plus `--manifest-fixture`. Keep status-fixture support.

- [ ] **Step 5: Run focused tests and production guard**

Run:

```bash
node --test scripts/sdk-submodule-guard.test.mjs
node scripts/check-sdk-submodule.mjs
```

Expected: all Node tests PASS; production guard prints `vendor Matrix SDK submodule path and gitlink are synced`.

- [ ] **Step 6: Commit**

```bash
git add scripts/sdk-submodule-guard.test.mjs scripts/lib/sdk-submodule-status.mjs scripts/check-sdk-submodule.mjs
git commit -m "build: enforce SDK submodule path dependencies"
```

---

### Task 2: Document the durable SDK build-input invariant

**Files:**
- Modify: `REPOSITORY_RULES.md`
- Modify: `AGENTS.md`

**Interfaces:**
- Consumes: `node scripts/check-sdk-submodule.mjs`
- Produces: one normative rule and one operational agent note

- [ ] **Step 1: Add the repository rule**

Add a `Matrix SDK source of truth` section stating:

```markdown
The `vendor/matrix-rust-sdk` gitlink is the only Matrix SDK revision pin.
All workspace Matrix SDK crates resolve through paths below that submodule.
Do not replace those paths with Git URL/revision dependencies; doing so can
compile code different from the SDK source under review.
```

- [ ] **Step 2: Add the AGENTS operational note**

Link to the durable rule and require these commands after SDK dependency or submodule changes:

```bash
node --test scripts/sdk-submodule-guard.test.mjs
node scripts/check-sdk-submodule.mjs
cargo metadata --no-deps --format-version 1
```

- [ ] **Step 3: Verify documentation and structure gates**

Run:

```bash
node --test scripts/sdk-submodule-guard.test.mjs scripts/build-structure-contract.test.mjs
git diff --check
```

Expected: PASS with no whitespace errors.

- [ ] **Step 4: Commit**

```bash
git add REPOSITORY_RULES.md AGENTS.md
git commit -m "docs: make the SDK submodule authoritative"
```

---

### Task 3: Freeze the exact anchored silent-gap behavior

**Files:**
- Modify: `vendor/matrix-rust-sdk/crates/matrix-sdk/src/event_cache/caches/room/live_tail.rs`
- Modify: `crates/koushi-core/src/bin/headless-core-qa.rs`

**Interfaces:**
- Consumes: tokenless `GET /rooms/{roomId}/messages?dir=b&limit=128`
- Produces: a loaded timeline containing `$latest`, `$missing`, and `$older` exactly once in chronological order

- [ ] **Step 1: Strengthen the SDK regression test**

Replace the count-only store materialization assertion with a decision-level test that proves an anchored response containing an in-store-only event cannot return `Unchanged`. Keep these core fixtures:

```rust
let cached = [owned_event_id!("$latest"), owned_event_id!("$older")];
let response = [
    owned_event_id!("$latest"),
    owned_event_id!("$missing"),
    owned_event_id!("$older"),
];
```

Assert anchor index `2`, materialized count `1`, and an `Advanced { events: 1 }` classification.

- [ ] **Step 2: Verify the strengthened SDK test fails against the old behavior**

Temporarily invoke the old newest-event truncation helper in the test fixture or revert only the reconciliation call, run the focused test, and confirm the assertion fails because `$missing` is discarded. Restore the current implementation immediately after observing RED.

Run:

```bash
cd vendor/matrix-rust-sdk
CARGO_TARGET_DIR=../../target cargo test -p matrix-sdk event_cache::caches::room::live_tail::tests --lib
```

- [ ] **Step 3: Add the real proxy page shape**

Extend the persisted-gap QA proxy with a page ordered newest-to-oldest as:

```rust
vec![
    QaCannedTimelineEvent {
        event_id: "$latest".to_owned(),
        sender: sender.to_owned(),
        body: latest_body.to_owned(),
        origin_server_ts: 3,
    },
    QaCannedTimelineEvent {
        event_id: "$missing".to_owned(),
        sender: sender.to_owned(),
        body: missing_body.to_owned(),
        origin_server_ts: 2,
    },
    QaCannedTimelineEvent {
        event_id: "$older".to_owned(),
        sender: sender.to_owned(),
        body: older_body.to_owned(),
        origin_server_ts: 1,
    },
]
```

Seed `$older` and `$latest` into A's persisted cache before restart, withhold `$missing`, then assert after tokenless refresh:

- `$missing` is emitted exactly once between the two anchors;
- one exact tokenless limit-128 request was served;
- the refresh diagnostic is `advanced`, never `unchanged`;
- no manual pagination command is required.

- [ ] **Step 4: Run short compile and unit gates**

Run:

```bash
cargo check -p koushi-core -p koushi-sdk
cd vendor/matrix-rust-sdk
CARGO_TARGET_DIR=../../target cargo test -p matrix-sdk event_cache::caches::room::live_tail::tests --lib
```

Expected: compile succeeds and all live-tail tests PASS.

- [ ] **Step 5: Commit**

Commit inside the SDK submodule first, then the superproject QA change and gitlink:

```bash
git -C vendor/matrix-rust-sdk add crates/matrix-sdk/src/event_cache/caches/room/live_tail.rs
git -C vendor/matrix-rust-sdk commit -m "test(event-cache): freeze anchored live-tail reconciliation"
git add vendor/matrix-rust-sdk crates/koushi-core/src/bin/headless-core-qa.rs
git commit -m "test(timeline): reproduce a silent gap before the live edge"
```

---

### Task 4: Consolidate unprojected gap selections without changing behavior

**Files:**
- Modify: `crates/koushi-core/src/timeline.rs`

**Interfaces:**
- Produces: `GapRepairSelection::Unprojected { ordinal, reason }`
- Produces: `UnprojectedGapReason::{LiveEdge, Foreground, Manual}`
- Preserves projected and direct committed-response selection

- [ ] **Step 1: Add characterization tests for all three reasons**

Write table-driven tests asserting:

```rust
assert_eq!(
    select_gap_repair_candidate(
        TimelineGapRepairTrigger::LiveEdge,
        &[],
        None,
        &[],
        4,
        true,
    ),
    GapRepairSelection::Unprojected {
        ordinal: 3,
        reason: UnprojectedGapReason::LiveEdge,
    },
);
assert_eq!(
    unlocated_gap_action(true, TimelineGapRepairTrigger::Automatic, 4, 0),
    UnlocatedGapAction::RepairNewest { ordinal: 3 },
);
assert_eq!(
    unlocated_gap_action(true, TimelineGapRepairTrigger::LiveTailSnapshot, 4, 0),
    UnlocatedGapAction::QueueAutomatic,
);
```

Also preserve Manual selecting the newest descriptor with reason `Manual`.

- [ ] **Step 2: Run the focused tests and verify RED**

Run:

```bash
cargo test -p koushi-core --lib timeline_gap_repair_tracker_tests
```

Expected: FAIL because the unified variants do not exist.

- [ ] **Step 3: Introduce the unified types**

Replace `LiveEdgeFallback` and `ManualFallback` with:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UnprojectedGapReason {
    LiveEdge,
    Foreground,
    Manual,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GapRepairSelection {
    None,
    Projected { id: TimelineGapId },
    DirectCommittedResponse,
    Unprojected { ordinal: usize, reason: UnprojectedGapReason },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UnlocatedGapAction {
    None,
    QueueAutomatic,
    RepairNewest { ordinal: usize },
}
```

Replace the duplicate `unlocated_gap_repair_ordinal` and `should_queue_unlocated_gap_repair` predicates with one exhaustive `unlocated_gap_action` function. Map `RepairNewest` to reason `Foreground`; derive `repaired_live_edge_fallback` only from reason `LiveEdge`.

- [ ] **Step 4: Remove obsolete branches and update diagnostics**

Delete all matches on `LiveEdgeFallback` and `ManualFallback`. Emit diagnostic outcomes `live_edge_fallback`, `foreground_unlocated`, or `manual_fallback` from the typed reason rather than from separate branches. Do not change budgets, projection fencing, or continuation rules.

- [ ] **Step 5: Run focused Core tests**

Run:

```bash
cargo test -p koushi-core --lib timeline_gap_repair_tracker_tests
cargo test -p koushi-core --lib gap_repair_room_switch_cancels_completion
cargo check -p koushi-core -p koushi-sdk
```

Expected: all tests PASS and compile succeeds.

- [ ] **Step 6: Commit**

```bash
git add crates/koushi-core/src/timeline.rs
git commit -m "refactor(timeline): unify unprojected gap selection"
```

---

### Task 5: Final integrated verification

**Files:**
- Verify only; fix only failures attributable to Tasks 1-4

**Interfaces:**
- Validates the complete SDK-source and recovery contract

- [ ] **Step 1: Run static and focused gates**

```bash
node --test scripts/sdk-submodule-guard.test.mjs scripts/build-structure-contract.test.mjs
node scripts/check-sdk-submodule.mjs
cargo fmt --all --check
cargo check -p koushi-core -p koushi-sdk
npm --prefix apps/desktop test -- --run src/domain/appStore.test.ts src/domain/orderedEventBatcher.test.ts
npm --prefix apps/desktop run typecheck
git diff --check
```

- [ ] **Step 2: Run the long recovery scenario once**

Run the repository's configured local homeserver scenario:

```bash
node scripts/desktop-headless-local-qa.mjs --run --server=conduit --scenario=timeline_legacy_persisted_gap --core --core-backend=legacy
```

Expected tokens include:

```text
legacy_live_tail_room_absent=ok
live_tail_detached_gap=ok
live_tail_historical_continuation=ok
```

and the new anchored silent-gap token.

- [ ] **Step 3: Review the final diff for scope**

Confirm:

- no guarded SDK Git URL or `rev` remains;
- no `readPinnedSdkRevision`, `LiveEdgeFallback`, `ManualFallback`, `unlocated_gap_repair_ordinal`, or `should_queue_unlocated_gap_repair` remains;
- causal projection and render-ACK behavior is unchanged;
- no unrelated generated or formatted file changed.
