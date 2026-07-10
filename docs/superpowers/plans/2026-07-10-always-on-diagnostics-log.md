# Always-On Unified Diagnostics Log Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Surface every application-owned diagnostic signal in the desktop Diagnostics dialog without requiring diagnostic environment variables.

**Architecture:** A new leaf crate, `koushi-diagnostics`, owns an always-on bounded structured ring buffer shared by `koushi-sdk`, `koushi-core`, and `koushi-desktop`. Existing environment variables remain only as stderr-mirroring switches. A read-only Tauri command snapshots the buffer, and React merges that snapshot with existing frontend diagnostics when opening the dialog.

**Tech Stack:** Rust 2024, `serde`, Tauri 2, TypeScript 6, React 19, Vitest, Cargo tests.

## Global Constraints

- Follow `docs/superpowers/specs/2026-07-10-always-on-diagnostics-log-design.md`.
- Preserve the existing uncommitted edits in `apps/desktop/src-tauri/src/commands/{live_signals.rs,mod.rs,timeline.rs}` and `crates/koushi-core/src/timeline.rs`; extend them instead of reverting or overwriting them.
- Diagnostic collection is always on. `KOUSHI_*TRACE` variables may control stderr mirroring only.
- Never collect raw Matrix identifiers, message/search text, filenames, local paths, URLs from room content, raw SDK errors, or secrets.
- The collector is telemetry-only: no `AppState`, reducer, or product-success decisions may depend on it.
- Do not capture arbitrary stdout/stderr and do not modify QA-only binary progress logging.
- Use test-driven development: add and run each focused failing test before production code.
- Luna implements one task at a time, self-reviews, runs the focused tests, and commits only that task's named files. Terra reviews the resulting commit range; the main integrator independently verifies accepted work.

---

## File Structure

- Create `crates/koushi-diagnostics/Cargo.toml`: leaf-crate manifest with only `serde`.
- Create `crates/koushi-diagnostics/src/lib.rs`: typed records, bounded buffer, global collector, stable formatter, and unit tests.
- Modify root `Cargo.toml`: add the new workspace/default member.
- Modify `crates/koushi-sdk/{Cargo.toml,src/lib.rs}`: record SDK-owned unread diagnostics.
- Modify `crates/koushi-core/Cargo.toml` and runtime trace files: record core diagnostics while retaining optional stderr mirroring.
- Modify `apps/desktop/src-tauri/{Cargo.toml,src/commands/diagnostics.rs,src/commands/mod.rs,src/commands/search.rs,src/lib.rs}`: record Tauri diagnostics and expose a snapshot command.
- Modify `apps/desktop/src/domain/{diagnostics.ts,diagnostics.test.ts}`: frontend snapshot types, merged formatting, dropped-count reporting, and always-on security diagnostics.
- Modify `apps/desktop/src/backend/{browserFakeApi.ts,browserFakeApi.test.ts,client.ts}`: add the read-only API method.
- Modify `apps/desktop/src/{App.tsx,App.test.tsx,vite-env.d.ts}`: fetch on open, merge records, remove verbose-env gating.

### Task 1: Add the structured diagnostics collector

**Files:**

- Create: `crates/koushi-diagnostics/Cargo.toml`
- Create: `crates/koushi-diagnostics/src/lib.rs`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`

**Interfaces:**

- Produces: `DiagnosticLevel`, `DiagnosticValue`, `DiagnosticField`, `DiagnosticEvent`, `DiagnosticRecord`, `DiagnosticSnapshot`, `DiagnosticBuffer`, `record`, `snapshot`, and `format_event`.
- `DiagnosticValue::Token`, `DiagnosticEvent.source`, `DiagnosticEvent.stage`, and field keys accept only `&'static str`; producers cannot store arbitrary runtime text.

- [ ] **Step 1: Write failing collector tests**

Add unit tests in `crates/koushi-diagnostics/src/lib.rs` for:

```rust
fn event(stage: &'static str) -> DiagnosticEvent {
    DiagnosticEvent::new(DiagnosticLevel::Debug, "test", stage)
}

#[test]
fn keeps_latest_records_and_reports_drops() {
    let buffer = DiagnosticBuffer::new(2);
    buffer.record_at(1, event("one"));
    buffer.record_at(2, event("two"));
    buffer.record_at(3, event("three"));

    let snapshot = buffer.snapshot();
    assert_eq!(snapshot.dropped_records, 1);
    assert_eq!(
        snapshot.records.iter().map(|record| record.event.stage).collect::<Vec<_>>(),
        vec!["two", "three"]
    );
}

#[test]
fn records_concurrently_without_exceeding_capacity() {
    let buffer = Arc::new(DiagnosticBuffer::new(64));
    let workers = (0..8)
        .map(|_| {
            let buffer = Arc::clone(&buffer);
            std::thread::spawn(move || {
                for index in 0..100 {
                    buffer.record_at(index, event("concurrent"));
                }
            })
        })
        .collect::<Vec<_>>();
    for worker in workers {
        worker.join().unwrap();
    }
    let snapshot = buffer.snapshot();
    assert_eq!(snapshot.records.len(), 64);
    assert_eq!(snapshot.dropped_records, 736);
}

#[test]
fn formats_only_structured_fields() {
    let line = format_event(&DiagnosticEvent::new(
        DiagnosticLevel::Debug,
        "core.timeline",
        "actor_finish",
    )
    .field(DiagnosticField::token("operation", "send_reaction"))
    .field(DiagnosticField::milliseconds("elapsed_ms", 42))
    .field(DiagnosticField::boolean("success", true)));
    assert_eq!(
        line,
        "stage=actor_finish operation=send_reaction elapsed_ms=42 success=true"
    );
}
```

- [ ] **Step 2: Run the new crate test and confirm RED**

Run: `cargo test -p koushi-diagnostics --lib`

Expected: FAIL because the crate/API does not exist.

- [ ] **Step 3: Implement the leaf crate and workspace wiring**

Use this public shape in `crates/koushi-diagnostics/src/lib.rs`:

```rust
pub const DEFAULT_DIAGNOSTIC_CAPACITY: usize = 10_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticLevel { Trace, Debug, Info, Warn, Error }

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum DiagnosticValue {
    Boolean(bool),
    Count(u64),
    Milliseconds(u64),
    RequestId { connection_id: u64, sequence: u64 },
    Token(&'static str),
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct DiagnosticField {
    pub key: &'static str,
    pub value: DiagnosticValue,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct DiagnosticEvent {
    pub level: DiagnosticLevel,
    pub source: &'static str,
    pub stage: &'static str,
    pub fields: Vec<DiagnosticField>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct DiagnosticRecord {
    #[serde(rename = "timestampMs")]
    pub timestamp_ms: u64,
    pub event: DiagnosticEvent,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct DiagnosticSnapshot {
    pub records: Vec<DiagnosticRecord>,
    #[serde(rename = "droppedRecords")]
    pub dropped_records: u64,
}
```

Implement these constructors:

```rust
impl DiagnosticEvent {
    pub fn new(
        level: DiagnosticLevel,
        source: &'static str,
        stage: &'static str,
    ) -> Self;
    pub fn field(self, field: DiagnosticField) -> Self;
}

impl DiagnosticField {
    pub fn token(key: &'static str, value: &'static str) -> Self;
    pub fn boolean(key: &'static str, value: bool) -> Self;
    pub fn count(key: &'static str, value: u64) -> Self;
    pub fn milliseconds(key: &'static str, value: u128) -> Self;
    pub fn request_id(
        key: &'static str,
        connection_id: u64,
        sequence: u64,
    ) -> Self;
}
```

Implement `DiagnosticBuffer` with a `Mutex<VecDeque<DiagnosticRecord>>`, a
separate dropped counter, best-effort poison handling, and a
`OnceLock<DiagnosticBuffer>` global. Clamp `u128` durations to `u64::MAX`.
`format_event` writes `stage` followed by fields in insertion order.

Add the crate to both `workspace.members` and `workspace.default-members`, add `serde = { version = "1", features = ["derive"] }`, and regenerate `Cargo.lock` through Cargo.

- [ ] **Step 4: Run collector tests and workspace formatting**

Run:

```bash
cargo test -p koushi-diagnostics --lib
cargo fmt --all -- --check
```

Expected: PASS.

- [ ] **Step 5: Luna task commit, then Terra review**

Review criteria: no arbitrary `String` diagnostic payload, no panic on poisoned lock/clock failure, capacity and drop count are exact.

Commit after Luna's self-review and focused verification, then send this commit range to Terra:

```bash
git add Cargo.toml Cargo.lock crates/koushi-diagnostics
git commit -m "feat: add bounded diagnostics collector"
```

### Task 2: Route non-timeline Rust diagnostics into the collector

**Files:**

- Modify: `crates/koushi-sdk/Cargo.toml`
- Modify: `crates/koushi-sdk/src/lib.rs`
- Modify: `crates/koushi-core/Cargo.toml`
- Modify: `crates/koushi-core/src/account.rs`
- Modify: `crates/koushi-core/src/room.rs`
- Modify: `crates/koushi-core/src/runtime.rs`
- Modify: `crates/koushi-core/src/search.rs`
- Modify: `crates/koushi-core/src/search_crawler.rs`
- Modify: `crates/koushi-core/src/startup_trace.rs`
- Modify: `crates/koushi-core/src/sync.rs`
- Modify: `crates/koushi-core/src/store.rs`

**Interfaces:**

- Consumes: `koushi_diagnostics::{record, DiagnosticEvent, DiagnosticField, DiagnosticLevel}`.
- Produces these stable sources: `sdk.unread`, `core.account`, `core.room`, `core.runtime`, `core.search`, `core.startup`, `core.sync`, `core.store`, and `core.intent`.

- [ ] **Step 1: Add failing environment-independent recording tests**

Add focused unit tests beside the trace helpers. Use a unique static stage token per test, call the helper with its environment variable removed, then assert `koushi_diagnostics::snapshot()` contains the token. Representative required tests:

```rust
#[test]
fn startup_phase_records_without_environment_switch() {
    trace_phase(StartupPhase::Restore, std::time::Instant::now());
    assert!(koushi_diagnostics::snapshot().records.iter().any(|record| {
        record.event.source == "core.startup" && record.event.stage == "restore"
    }));
}

#[test]
fn room_operation_records_without_environment_switch() {
    trace_room_operation("create_room", "test_always_on", make_request_id(999));
    assert!(koushi_diagnostics::snapshot().records.iter().any(|record| {
        record.event.source == "core.room" && record.event.stage == "test_always_on"
    }));
}
```

The test bodies do not mutate process environment. The test commands below
remove the variables before the test process starts.

- [ ] **Step 2: Run focused tests and confirm RED**

Run:

```bash
env -u KOUSHI_STARTUP_TRACE cargo test -p koushi-core --lib startup_phase_records_without_environment_switch
env -u KOUSHI_CORE_ACTOR_TRACE cargo test -p koushi-core --lib room_operation_records_without_environment_switch
```

Expected: FAIL because helpers return before recording.

- [ ] **Step 3: Add dependencies and split capture from stderr mirroring**

Add `koushi-diagnostics = { path = "../koushi-diagnostics" }` to SDK/core manifests. Every helper follows this pattern:

```rust
fn trace_room_operation(
    kind: &'static str,
    stage: &'static str,
    request_id: RequestId,
) {
    record(
        DiagnosticEvent::new(DiagnosticLevel::Debug, "core.room", stage)
            .field(DiagnosticField::token("operation", kind))
            .field(DiagnosticField::request_id(
                "request_id",
                request_id.connection_id.0,
                request_id.sequence,
            )),
    );
    if std::env::var_os("KOUSHI_CORE_ACTOR_TRACE").is_some() {
        eprintln!(
            "koushi_core actor_trace room_actor stage={stage} kind={kind} request_id={}/{}",
            request_id.connection_id.0,
            request_id.sequence
        );
    }
}
```

Migrate the existing families using the following exact source/stage policy:

| Existing family/site | Collector source | Structured fields |
|---|---|---|
| SDK unread helpers in `koushi-sdk/src/lib.rs` | `sdk.unread` | room kind/counts, booleans; never room id |
| restore/account routing helpers in `account.rs` | `core.account` | operation/action token, request id, booleans |
| `trace_room_operation` in `room.rs` | `core.room` | operation, request id |
| `trace_runtime_sync`, app-loop latency, `IntentLifecycle` emission in `runtime.rs` | `core.runtime` / `core.intent` | action/outcome token, request id, counts, durations |
| search query/crawl helpers in `search.rs` | `core.search` | scope kind, dropped/processed/indexed counts, durations |
| `startup_trace.rs` and `search_crawler.rs` | `core.startup` | phase/origin/outcome token, item counts, durations |
| `trace_sync!` call sites in `sync.rs` | `core.sync` | operation/status/outcome token, request id, booleans, counts |
| store diagnostic helper in `store.rs` | `core.store` | operation/outcome token only; never path or raw error |

Replace call-site `format!`/`format_args!` diagnostic payloads with typed fields. Keep stderr text compatible where QA scripts parse it. Change helper `&str` parameters that only accept literals/enums to `&'static str`.

Startup timing and origin observation must run even when
`KOUSHI_STARTUP_TRACE` is unset: replace `now_if_enabled()` with
`now() -> std::time::Instant`, update trace functions to accept `Instant`, and
split `enabled()` into always-on collector observation plus a separate
`stderr_enabled()` used only around `eprintln!`.

- [ ] **Step 4: Add privacy regression assertions**

Using synthetic inputs containing `!room:example.invalid`, `@user:example.invalid`, `$event:example.invalid`, `/Users/alice/private`, and `secret message`, serialize `koushi_diagnostics::snapshot()` and assert none appear. Assert the expected kind/count/presence token does appear.

- [ ] **Step 5: Run focused and crate-level tests**

Run:

```bash
cargo test -p koushi-sdk --lib
cargo test -p koushi-core --lib startup_trace
cargo test -p koushi-core --lib sync_trace
cargo test -p koushi-core --lib restore_trace
cargo test -p koushi-core --lib diagnostic
```

Expected: PASS with stderr compatibility tests unchanged.

- [ ] **Step 6: Luna task commit, then Terra review**

Luna commits the task files below. Terra then reviews every `record(` call for private inputs and confirms environment checks wrap only `eprintln!`, never `record`.

```bash
git add crates/koushi-sdk crates/koushi-core Cargo.lock
git commit -m "feat: collect core diagnostics without env gates"
```

### Task 3: Route timeline and unread diagnostics, preserving current latency work

**Files:**

- Modify: `crates/koushi-core/src/timeline.rs`
- Modify: `crates/koushi-core/src/unread_trace.rs`

**Interfaces:**

- Consumes: the Task 1 collector.
- Produces stable sources `core.timeline`, `core.timeline_item`, `core.event_cache`, and `core.unread`.

- [ ] **Step 1: Add failing tests for the current reaction/read latency helpers**

Extend the existing trace tests to execute `trace_timeline_actor_operation`, `trace_timeline_actor_scan`, `trace_timeline_route`, and `trace_mark_read` with `KOUSHI_SUBSCRIBE_TRACE`, `KOUSHI_TIMELINE_ITEM_TRACE`, and `KOUSHI_UNREAD_TRACE` unset. Assert records contain operation/stage/count/duration/outcome fields and contain no supplied room/event ids.

- [ ] **Step 2: Run the focused tests and confirm RED**

Run:

```bash
cargo test -p koushi-core --lib reaction_and_read_signal_handlers_emit_private_latency_traces
cargo test -p koushi-core --lib timeline_item_trace_line_is_private_data_free
cargo test -p koushi-core --lib activity_trace_lines_are_deduped_by_full_line
```

Expected: at least the new collector assertions FAIL.

- [ ] **Step 3: Record all timeline trace families before optional stderr**

Update these existing helpers, without deleting the current uncommitted latency instrumentation:

- `trace_timeline_actor_operation`
- `trace_timeline_actor_scan`
- `trace_timeline_route`
- `trace_timeline_paginate`
- `trace_timeline_link_preview`
- `trace_timeline_items`
- `trace_timeline_diffs`
- `trace_event_cache_items`
- `trace_event_cache_diffs`
- `trace_mark_read`, room-list, and activity helpers in `unread_trace.rs`

For item/cache traces, collect only row kind, diff operation, index/count, origin/relation kind, timestamp bucket, booleans, and aggregate counts. Do not collect event/sender ids even if the legacy stderr formatter hashes or redacts them. Replace `timeline_trace_enabled().then(Instant::now)` with always-on timing where the elapsed value is part of the diagnostic record; keep `timeline_trace_enabled()` for stderr only.

- [ ] **Step 4: Run timeline/unread tests and formatting**

Run:

```bash
cargo test -p koushi-core --lib timeline_trace
cargo test -p koushi-core --lib unread_trace
cargo test -p koushi-core --lib reaction_and_read_signal_handlers_emit_private_latency_traces
cargo fmt --all -- --check
```

Expected: PASS.

- [ ] **Step 5: Luna task commit, then Terra review**

Luna commits the task files below. Terra then reviews against the pre-task dirty diff to prove all pre-existing latency additions remain present.

```bash
git add crates/koushi-core/src/timeline.rs crates/koushi-core/src/unread_trace.rs
git commit -m "feat: collect timeline diagnostics without env gates"
```

### Task 4: Expose the Rust/Tauri diagnostic snapshot

**Files:**

- Modify: `apps/desktop/src-tauri/Cargo.toml`
- Create: `apps/desktop/src-tauri/src/commands/diagnostics.rs`
- Modify: `apps/desktop/src-tauri/src/commands/mod.rs`
- Modify: `apps/desktop/src-tauri/src/commands/search.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src/backend/browserFakeApi.ts`
- Modify: `apps/desktop/src/backend/browserFakeApi.test.ts`
- Modify: `apps/desktop/src/backend/client.ts`
- Modify: `apps/desktop/src/domain/diagnostics.ts`

**Interfaces:**

- Produces `DesktopApi.getDiagnosticSnapshot(): Promise<DiagnosticLogSnapshot>`.
- Produces Tauri command `get_diagnostic_snapshot`.
- Produces frontend types `DiagnosticLogSnapshot` and `DiagnosticLogEntry`.

- [ ] **Step 1: Add failing API and command tests**

Add a browser fake test:

```typescript
test("returns an empty diagnostic snapshot in the browser fake", async () => {
  const api = createBrowserFakeApi();
  await expect(api.getDiagnosticSnapshot()).resolves.toEqual({
    entries: [],
    droppedEntries: 0
  });
});
```

Add a Rust command/registration test asserting the command maps a structured collector record to camelCase frontend output and is present in `tauri::generate_handler!`.

- [ ] **Step 2: Run focused tests and confirm RED**

Run:

```bash
npm --prefix apps/desktop test -- src/backend/browserFakeApi.test.ts
cargo test -p koushi-desktop diagnostic_snapshot
```

Expected: FAIL because the method and command do not exist.

- [ ] **Step 3: Implement the read-only bridge**

Add `koushi-diagnostics = { path = "../../../crates/koushi-diagnostics" }` to the Tauri manifest. In `commands/diagnostics.rs`, map each structured record using `koushi_diagnostics::format_event`:

```rust
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendDiagnosticLogEntry {
    timestamp_ms: u64,
    source: &'static str,
    message: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendDiagnosticLogSnapshot {
    entries: Vec<FrontendDiagnosticLogEntry>,
    dropped_entries: u64,
}

#[tauri::command]
pub fn get_diagnostic_snapshot() -> FrontendDiagnosticLogSnapshot {
    let snapshot = koushi_diagnostics::snapshot();
    FrontendDiagnosticLogSnapshot {
        entries: snapshot.records.into_iter().map(|record| FrontendDiagnosticLogEntry {
            timestamp_ms: record.timestamp_ms,
            source: record.event.source,
            message: koushi_diagnostics::format_event(&record.event),
        }).collect(),
        dropped_entries: snapshot.dropped_records,
    }
}
```

Register the module/command. In TypeScript add:

```typescript
export interface DiagnosticLogSnapshot {
  entries: DiagnosticLogEntry[];
  droppedEntries: number;
}
```

Add the method to `DesktopApi`, return the empty snapshot in `BrowserFakeApi`, and invoke `get_diagnostic_snapshot` in `TauriDesktopApi`.

Update Tauri search/timeline helpers so they record structured entries before their existing stderr gates.

- [ ] **Step 4: Run bridge tests and typecheck**

Run:

```bash
cargo test -p koushi-desktop diagnostic_snapshot
npm --prefix apps/desktop test -- src/backend/browserFakeApi.test.ts
npm --prefix apps/desktop run typecheck
```

Expected: PASS.

- [ ] **Step 5: Luna task commit, then Terra review**

Luna commits the task files below. Terra then reviews for camelCase contract accuracy, no product state mutation, and no raw structured values formatted in Tauri.

```bash
git add apps/desktop/src-tauri apps/desktop/src/backend apps/desktop/src/domain/diagnostics.ts Cargo.lock
git commit -m "feat: expose desktop diagnostic snapshot"
```

### Task 5: Merge all records in Diagnostics and remove verbose gating

**Files:**

- Modify: `apps/desktop/src/domain/diagnostics.ts`
- Modify: `apps/desktop/src/domain/diagnostics.test.ts`
- Modify: `apps/desktop/src/App.tsx`
- Modify: `apps/desktop/src/App.test.tsx`
- Modify: `apps/desktop/src/vite-env.d.ts`

**Interfaces:**

- Consumes: `DesktopApi.getDiagnosticSnapshot()`.
- `diagnosticReport` consumes `logEntries`, `droppedLogEntries`, and `securityDiagnostics`.
- Diagnostics opening is asynchronous but the dialog still opens on a coarse fetch failure.

- [ ] **Step 1: Replace the verbose-gate test with failing always-on tests**

In `diagnostics.test.ts`, keep the current verbose-gating test's fully populated
`baseInput` constant, rename the test, and replace the two report calls and
assertions with:

```typescript
test("always includes security and merged runtime diagnostics", async () => {
  const report = diagnosticReport({
    ...baseInput,
    securityDiagnostics: {
      secureContext: true,
      locationProtocol: "http:",
      locationOrigin: "http://localhost:5173",
      avatarImageSchemes: { asset: 3 },
      avatarBrokenImages: 1
    },
    droppedLogEntries: 7,
    logEntries: [
      { timestampMs: 2, source: "frontend.timeline", message: "stage=render" },
      { timestampMs: 1, source: "core.timeline", message: "stage=actor_start" }
    ]
  });
  expect(report).toContain("Security diagnostics:");
  expect(report).toContain("Diagnostic records dropped: 7");
  expect(report.indexOf("core.timeline")).toBeLessThan(report.indexOf("frontend.timeline"));
  expect(report).not.toContain("Verbose diagnostics: disabled");
});
```

Add an `App.test.tsx` contract proving `onOpenDiagnostics` calls an async `openDiagnostics`, that the result is stored, and that fetch failure records only `source=diagnostics.fetch message=kind=unavailable`.

- [ ] **Step 2: Run focused frontend tests and confirm RED**

Run:

```bash
npm --prefix apps/desktop test -- src/domain/diagnostics.test.ts src/App.test.tsx
```

Expected: FAIL because security is still gated and runtime snapshots are not fetched/merged.

- [ ] **Step 3: Implement asynchronous open and merged report formatting**

In `App.tsx`:

```typescript
const [runtimeDiagnosticSnapshot, setRuntimeDiagnosticSnapshot] =
  useState<DiagnosticLogSnapshot>({ entries: [], droppedEntries: 0 });

async function openDiagnostics() {
  try {
    setRuntimeDiagnosticSnapshot(await api.getDiagnosticSnapshot());
  } catch {
    setRuntimeDiagnosticSnapshot({
      entries: [{
        timestampMs: Date.now(),
        source: "diagnostics.fetch",
        message: "kind=unavailable"
      }],
      droppedEntries: 0
    });
  }
  setDiagnosticsOpen(true);
}
```

Wire `onOpenDiagnostics={() => { void openDiagnostics(); }}`. Pass
`logEntries={[...runtimeDiagnosticSnapshot.entries, ...diagnosticLogEntries]}`,
`droppedLogEntries={runtimeDiagnosticSnapshot.droppedEntries}`, and
`securityDiagnostics={qaSecurityDiagnostics()}` to `diagnosticReport`.

In `diagnostics.ts`, replace `VerboseDiagnostics`/`formatVerboseDiagnostics` with an optional `SecurityDiagnostics` input that always formats when present. Rename `"Timeline log:"` to `"Diagnostic log:"` and add `Diagnostic records dropped: N` before the chronological entries.

Delete `verboseDiagnosticsEnabled()` and remove `VITE_KOUSHI_VERBOSE_DIAGNOSTICS` from `vite-env.d.ts`.

- [ ] **Step 4: Run frontend tests, typecheck, and lint**

Run:

```bash
npm --prefix apps/desktop test -- src/domain/diagnostics.test.ts src/backend/browserFakeApi.test.ts src/App.test.tsx
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run lint
```

Expected: PASS.

- [ ] **Step 5: Luna task commit, then Terra review**

Luna commits the task files below. Terra then reviews that the dialog opens after both success and failure, raw caught errors are not recorded, and security diagnostics are present without Vite environment state.

```bash
git add apps/desktop/src/domain/diagnostics.ts apps/desktop/src/domain/diagnostics.test.ts apps/desktop/src/App.tsx apps/desktop/src/App.test.tsx apps/desktop/src/vite-env.d.ts
git commit -m "feat: show unified diagnostics without env gates"
```

### Task 6: Complete the trace inventory and run repository gates

**Files:**

- Modify: `apps/desktop/src/scripts/releaseScripts.test.ts`
- Audit only: the runtime source files migrated in Tasks 2–5.
- Do not modify `crates/koushi-core/src/bin/*`, `crates/koushi-sdk/src/bin/*`, or QA runner progress output.

**Interfaces:**

- Produces a zero-result application-runtime inventory for environment checks that suppress collector recording.

- [ ] **Step 1: Run the exact inventory and inspect every result**

Run:

```bash
rg -n "KOUSHI_[A-Z0-9_]*(TRACE|DIAGNOST)|VITE_KOUSHI_VERBOSE_DIAGNOSTICS" \
  crates/koushi-sdk/src crates/koushi-core/src apps/desktop/src-tauri/src apps/desktop/src \
  --glob '!**/bin/**'
```

For every result, prove one of:

1. it controls stderr mirroring only and `record(...)` executes first;
2. it is a test asserting stderr compatibility;
3. it is documentation/type removal covered by Task 5.

- [ ] **Step 2: Add a structural regression test**

Add a repository script/unit test that scans application runtime Rust sources and fails when an `if std::env::*TRACE` block contains the only call to a trace helper or `record`. The permitted shape is `record(...); if env_is_set { eprintln!(...) }`.

- [ ] **Step 3: Run full relevant verification**

Run:

```bash
cargo fmt --all -- --check
cargo test -p koushi-diagnostics --lib
cargo test -p koushi-sdk --lib
cargo test -p koushi-core --lib
cargo test -p koushi-desktop
npm --prefix apps/desktop test
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run lint
npm --prefix apps/desktop run lint:tauri-boundary
npm --prefix apps/desktop run lint:domain-deps
npm --prefix apps/desktop run qa:secret-scan
npm --prefix apps/desktop run qa:release-gates -- --no-compile
```

Expected: every command exits 0 with no privacy leak output.

- [ ] **Step 4: Terra final diff review**

Review against `REPOSITORY_RULES.md`, `docs/architecture/overview.md`, `docs/policies/engineering-rules.md`, and the design spec. Priorities: privacy, environment-independent coverage, no product-lane coupling, preservation of the pre-existing dirty latency changes.

- [ ] **Step 5: Luna task commit, Terra review, and main-integrator verification**

Luna commits the structural inventory test after all earlier task diffs are
already reviewed and committed. Terra reviews it before the main integrator
runs the final verification:

```bash
git add apps/desktop/src/scripts/releaseScripts.test.ts
git commit -m "test: enforce always-on diagnostic collection"
```

Do not commit unrelated `HANDOFF.md`, `docs/design/sidebar-dm-rooms-sort-mock.svg`, or `log` files.
