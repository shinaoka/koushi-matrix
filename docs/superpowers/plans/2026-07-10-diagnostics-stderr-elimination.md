# Diagnostics stderr-elimination Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove every application-runtime diagnostic stderr mirror and record schema-contract mismatches through the sanitized Diagnostics report.

**Architecture:** The existing `koushi_diagnostics` collector remains the only Rust/Tauri diagnostic sink. The frontend creates a fixed, private-data-free `snapshot` entry at the rejected IPC boundary. A source-level release test rejects future runtime `eprintln!` calls and trace environment literals, while excluding test modules and QA binaries.

**Tech Stack:** Rust 2024, Tauri 2, TypeScript 6, React 19, Vitest, Cargo.

## Global Constraints

- Follow `docs/superpowers/specs/2026-07-10-diagnostics-stderr-elimination-design.md`.
- Modify application runtime only; retain CLI/CI/QA script reporting and test-only output.
- Keep structured Diagnostics collection always-on and private-data-free.
- Do not capture arbitrary stderr, SDK, dependency, browser, or operating-system output.
- Use test-driven development: run every named focused test red before changing production code, then green after the minimal change.

---

## File structure

- Modify `apps/desktop/src/scripts/releaseScripts.test.ts`: reject production runtime `eprintln!` and all former diagnostic env literals.
- Modify `crates/koushi-sdk/src/lib.rs`: remove unread stderr formatting/mirror helpers while retaining its structured unread record.
- Modify `crates/koushi-core/src/{startup_trace.rs,unread_trace.rs,timeline.rs,runtime.rs,sync.rs,account.rs,room.rs,search.rs,store.rs}`: remove trace gates and eprintln branches, retaining existing `record`/`record_batch` calls.
- Modify `apps/desktop/src-tauri/src/commands/{mod.rs,search.rs}`: remove Tauri trace gates and mirrors after unconditional collector calls.
- Modify `apps/desktop/src/domain/{diagnostics.ts,diagnostics.test.ts}`: add a fixed schema-mismatch entry factory.
- Modify `apps/desktop/src/{App.tsx,App.diagnostics.test.tsx}`: append the schema-mismatch entry and prove it appears without console output.
- Modify `scripts/desktop-real-homeserver-qa.mjs` and `apps/desktop/src/scripts/releaseScripts.test.ts` only if the old startup-trace env is forwarded solely to the removed mirror.

### Task 1: Lock and implement the no-runtime-stderr contract

**Files:**

- Modify: `apps/desktop/src/scripts/releaseScripts.test.ts`
- Modify: `crates/koushi-sdk/src/lib.rs`
- Modify: `crates/koushi-core/src/startup_trace.rs`
- Modify: `crates/koushi-core/src/unread_trace.rs`
- Modify: `crates/koushi-core/src/timeline.rs`
- Modify: `crates/koushi-core/src/runtime.rs`
- Modify: `crates/koushi-core/src/sync.rs`
- Modify: `crates/koushi-core/src/account.rs`
- Modify: `crates/koushi-core/src/room.rs`
- Modify: `crates/koushi-core/src/search.rs`
- Modify: `crates/koushi-core/src/store.rs`
- Modify: `apps/desktop/src-tauri/src/commands/mod.rs`
- Modify: `apps/desktop/src-tauri/src/commands/search.rs`
- Modify: `scripts/desktop-real-homeserver-qa.mjs`

**Interfaces:**

- Produces: `runtimeDiagnosticStderrFindings(sources: DiagnosticSource[]): DiagnosticGateFinding[]`.
- Input: `runtimeRustSources()` and `productionRustLines()` already in this test module.
- Output: a finding with `reason = "runtime diagnostic writes to stderr"` for every production `eprintln!` and with `reason = "runtime diagnostic environment gate remains"` for the listed legacy variables.

- [ ] **Step 1: Write the failing source-contract test**

Add this test immediately after the current release-scanner tests:

```ts
test("application runtime has no diagnostic stderr mirror or trace environment gate", () => {
  expect(runtimeDiagnosticStderrFindings(runtimeRustSources())).toEqual([]);
});
```

Add the explicit legacy set:

```ts
const REMOVED_DIAGNOSTIC_ENV_LITERALS = [
  "KOUSHI_STARTUP_TRACE",
  "KOUSHI_SUBSCRIBE_TRACE",
  "KOUSHI_TIMELINE_ITEM_TRACE",
  "KOUSHI_UNREAD_TRACE",
  "KOUSHI_SEARCH_TRACE",
  "KOUSHI_SYNC_TRACE",
  "KOUSHI_CORE_ACTOR_TRACE",
  "KOUSHI_DEBUG_SDK_ERROR"
];
```

- [ ] **Step 2: Verify RED**

Run: `npm --prefix apps/desktop test -- src/scripts/releaseScripts.test.ts`

Expected: FAIL with findings for production `eprintln!` and the former trace environment variables.

- [ ] **Step 3: Add the smallest scanner helper**

Use the already lexicalized production lines so comments, strings, and `#[cfg(test)]` sections do not count:

```ts
function runtimeDiagnosticStderrFindings(sources: DiagnosticSource[]): DiagnosticGateFinding[] {
  return sources.flatMap(({ relativePath, source }) =>
    productionRustLines(source).flatMap((line, index) => {
      const reason = /\\beprintln!\\s*\\(/.test(line)
        ? "runtime diagnostic writes to stderr"
        : REMOVED_DIAGNOSTIC_ENV_LITERALS.some((literal) => line.includes(literal))
          ? "runtime diagnostic environment gate remains"
          : null;
      return reason
        ? [{ relativePath, line: index + 1, location: `${relativePath}:${index + 1}`, reason }]
        : [];
    })
  );
}
```

- [ ] **Step 4: Remove the mirror-only code**

For every listed runtime helper, delete only the `if <trace-enabled> { eprintln!(...) }` block, trace boolean, and helper/constant that become unused. Preserve the preceding structured collector call exactly. For item/diff loops, delete the whole stderr-only formatting loop after `record_batch(events)`; do not change event construction.

Remove `KOUSHI_STARTUP_TRACE: "1"` from the real-homeserver QA child environment because its only effect was the deleted startup stderr mirror.

- [ ] **Step 5: Verify GREEN**

Run: `npm --prefix apps/desktop test -- src/scripts/releaseScripts.test.ts`

Expected: PASS; the production source contains no `eprintln!` or listed diagnostic environment literal.

- [ ] **Step 6: Run focused Rust verification**

Run:

```bash
cargo test -p koushi-sdk
cargo test -p koushi-core
cargo test -p koushi-desktop
cargo fmt --all -- --check
```

Expected: PASS, including environment-unset collector tests.

### Task 2: Capture schema mismatches in Diagnostics

**Files:**

- Modify: `apps/desktop/src/domain/diagnostics.ts`
- Modify: `apps/desktop/src/domain/diagnostics.test.ts`
- Modify: `apps/desktop/src/App.tsx`
- Modify: `apps/desktop/src/App.diagnostics.test.tsx`

**Interfaces:**

- Produces: `schemaMismatchDiagnosticEntry(timestampMs: number): DiagnosticLogEntry`.
- Contract: `{ source: "snapshot", message: "schema_mismatch" }`; it contains neither expected nor received schema version and no IPC payload value.

- [ ] **Step 1: Write the failing domain test**

```ts
test("creates a fixed private-data-free schema mismatch diagnostic", () => {
  expect(schemaMismatchDiagnosticEntry(42)).toEqual({
    timestampMs: 42,
    source: "snapshot",
    message: "schema_mismatch"
  });
});
```

- [ ] **Step 2: Verify RED**

Run: `npm --prefix apps/desktop test -- src/domain/diagnostics.test.ts`

Expected: FAIL because `schemaMismatchDiagnosticEntry` is not exported.

- [ ] **Step 3: Add the minimum domain factory and use it at the boundary**

```ts
export function schemaMismatchDiagnosticEntry(timestampMs: number): DiagnosticLogEntry {
  return { timestampMs, source: "snapshot", message: "schema_mismatch" };
}
```

Import it in `App.tsx`. Replace the `console.error` call in `setSnapshot` with:

```ts
setDiagnosticLogEntries((current) =>
  appendDiagnosticLogEntry(current, schemaMismatchDiagnosticEntry(Date.now()))
);
```

Keep `setSchemaMismatchVersion` and the fail-closed recovery UI unchanged.

- [ ] **Step 4: Add the UI regression assertion and verify GREEN**

Make a fake API return a snapshot with a mismatched `schema_version`; assert the recovery alert renders, `console.error` is not called, and a Diagnostics report opened after a compatible refresh contains `snapshot schema_mismatch` without the numeric schema value.

Run:

```bash
npm --prefix apps/desktop test -- src/domain/diagnostics.test.ts src/App.diagnostics.test.tsx
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run lint
```

Expected: PASS.

### Task 3: Final release verification and PR preparation

**Files:**

- Modify: `.superpowers/sdd/progress.md`

- [ ] **Step 1: Run the full gate set**

```bash
npm --prefix apps/desktop test
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run lint
npm --prefix apps/desktop run lint:tauri-boundary
npm --prefix apps/desktop run lint:domain-deps
npm --prefix apps/desktop run qa:secret-scan
npm --prefix apps/desktop run qa:release-gates -- --no-compile
cargo test -p koushi-diagnostics
cargo test -p koushi-sdk
cargo test -p koushi-core
cargo test -p koushi-desktop
cargo fmt --all -- --check
git diff --check
```

Expected: all commands succeed.

- [ ] **Step 2: Record evidence and commit**

Add the focused and full verification counts to `.superpowers/sdd/progress.md`, preserving the pre-existing `task-3-report.md` modification. Commit only the stderr-elimination implementation, tests, plan, and progress document:

```bash
git add docs/superpowers apps/desktop/src crates/koushi-sdk/src crates/koushi-core/src scripts/desktop-real-homeserver-qa.mjs
git commit -m "fix: route diagnostics exclusively through collector"
```

- [ ] **Step 3: Push and open a PR**

Push `codex/always-on-diagnostics`, inspect the default remote base branch, and create a draft PR with the full always-on Diagnostics feature plus stderr-elimination amendment. Do not stage `.superpowers/sdd/task-3-report.md`.
