# Task 6 — Always-on unified diagnostics

## Status

Complete. The runtime inventory is clean: the structural scanner reports no
gated-only diagnostic producers in the SDK, core, or Tauri runtime sources.

## Commits

- `08f60e0` — `fix: collect event cache repair diagnostics without trace gates`
- `c0b0b45` — `test: enforce always-on diagnostic collection`

## Changed files

- `crates/koushi-core/src/account.rs`
- `apps/desktop/src/scripts/releaseScripts.test.ts`
- `.superpowers/sdd/task-6-report.md`

The pre-existing dirty `.superpowers/sdd/task-3-report.md` was not edited or
staged.

## Exact inventory

The required command was run after the fix. It returned 57 matches:

```text
rg -n "KOUSHI_[A-Z0-9_]*(TRACE|DIAGNOST)|VITE_KOUSHI_VERBOSE_DIAGNOSTICS" \
  crates/koushi-sdk/src crates/koushi-core/src apps/desktop/src-tauri/src apps/desktop/src \
  --glob '!**/bin/**'
```

Classification of every result:

1. Stderr mirror gates with collection first — 15 results:

   - `apps/desktop/src-tauri/src/commands/mod.rs:117,141,696`
   - `crates/koushi-core/src/room.rs:2745`
   - `crates/koushi-core/src/account.rs:1078,1079,4946,4961`
   - `crates/koushi-core/src/timeline.rs:316,885,956,3110,5429`
   - `crates/koushi-core/src/runtime.rs:125,1139`

   The `account.rs:1078-1079` boolean alias is now followed by structured
   collection on every cache-repair outcome before the unchanged stderr mirror.

2. Test-only environment or compatibility assertions — 27 results:

   - `apps/desktop/src-tauri/src/commands/mod.rs:7055,7056,7171,7172`
   - `crates/koushi-core/src/sync.rs:1963`
   - `crates/koushi-core/src/account.rs:5627,5628,5640,5641,5933`
   - `crates/koushi-core/src/timeline.rs:10464,10465,10466,10467,10480,10481,10482,10483`
   - `apps/desktop/src/scripts/releaseScripts.test.ts:317,326,334,341,355,1142,1154,1565`
   - `crates/koushi-core/src/runtime.rs:4334`

   The first five release-test locations are synthetic scanner fixtures; the
   remaining three are existing QA/test environment contracts. These are not
   runtime producers.

3. Comments, constants, or helpers consumed by category 1 — 14 results:

   - `apps/desktop/src-tauri/src/commands/search.rs:6`
   - `apps/desktop/src-tauri/src/commands/mod.rs:691`
   - `crates/koushi-sdk/src/lib.rs:55`
   - `crates/koushi-core/src/search.rs:75`
   - `crates/koushi-core/src/unread_trace.rs:10`
   - `crates/koushi-core/src/sync.rs:75`
   - `crates/koushi-core/src/account.rs:89`
   - `crates/koushi-core/src/timeline.rs:881,1566,1784`
   - `crates/koushi-core/src/startup_trace.rs:4,44`
   - `crates/koushi-core/src/runtime.rs:80,109`

4. Task 5 removed-Vite-variable assertion — 1 result:

   - `apps/desktop/src/App.diagnostics.test.tsx:199`

The scanner separately resolves indirect gate styles not visible as literal
environment names on the gate line, including `search_trace_enabled()`,
`stderr_enabled()`, `enabled()`, `let trace = ...`, and trace-module helpers.
The post-fix runtime scan returned no findings.

## TDD evidence

### Structural RED

The first structural scanner test was run before the production fix:

```text
npm --prefix apps/desktop test -- src/scripts/releaseScripts.test.ts -t "always-on diagnostic collection rejects trace-only producers and accepts stderr mirrors"
```

Result: exit 1; 1 failed, 121 skipped, 122 total. The scanner reported the six
gated-only cache-repair branches:

```text
crates/koushi-core/src/account.rs:1063
crates/koushi-core/src/account.rs:1069
crates/koushi-core/src/account.rs:1075
crates/koushi-core/src/account.rs:1081
crates/koushi-core/src/account.rs:1089
crates/koushi-core/src/account.rs:1094
```

The synthetic bad fixture failed for the same reason, while the good stderr
mirror fixture and test-only fixture assertions were retained as non-vacuous
checks.

### Producer RED

The focused producer test was added before the implementation and run with the
trace variables removed in the child process:

```text
CARGO_TARGET_DIR=/Users/hiroshi/projects/Element-dev/matrix-desktop/target cargo test -p koushi-core --lib event_cache_repair_diagnostic_runs_without_trace_environment
```

Result before the fix: exit 101. The child assertion failed because no
`core.event_cache_repair` record existed.

### GREEN

After the fix, the same focused Rust command passed both the outer child-launch
test and its ignored env-unset child test. The same structural scanner command
passed 1 test with 121 skipped tests.

## Production gap found and fixed

`AccountActor::handle_ensure_room_event_cached` had six outcomes whose only
diagnostic output was behind the combined
`KOUSHI_TIMELINE_ITEM_TRACE || KOUSHI_SUBSCRIBE_TRACE` boolean alias:

- no session
- invalid room
- invalid event
- missing room
- SDK load success
- SDK load failure

The fix adds `core.event_cache_repair` records before each optional stderr
mirror. Records contain only a typed request ID and fixed `stage`, `outcome`,
and `reason` tokens. Existing stderr text and environment behavior are
unchanged. The focused test invokes the actual actor message path with both
trace variables absent and asserts the typed, private-data-free record.

## Verification

All required commands completed with exit status 0:

```text
PASS  cargo fmt --all -- --check
PASS  CARGO_TARGET_DIR=/Users/hiroshi/projects/Element-dev/matrix-desktop/target cargo test -p koushi-diagnostics --lib — 8 passed, 0 failed
PASS  CARGO_TARGET_DIR=/Users/hiroshi/projects/Element-dev/matrix-desktop/target cargo test -p koushi-sdk --lib — 43 passed, 0 failed
PASS  CARGO_TARGET_DIR=/Users/hiroshi/projects/Element-dev/matrix-desktop/target cargo test -p koushi-core --lib — 418 passed, 1 ignored, 0 failed
PASS  CARGO_TARGET_DIR=/Users/hiroshi/projects/Element-dev/matrix-desktop/target cargo test -p koushi-desktop — 96 lib tests passed, 5 integration tests passed, 1 ignored; 0 failed
PASS  npm --prefix apps/desktop test — 46 files passed, 715 tests passed
PASS  npm --prefix apps/desktop run typecheck
PASS  npm --prefix apps/desktop run lint
PASS  npm --prefix apps/desktop run lint:tauri-boundary
PASS  npm --prefix apps/desktop run lint:domain-deps
PASS  npm --prefix apps/desktop run qa:secret-scan
PASS  npm --prefix apps/desktop run qa:release-gates -- --no-compile
PASS  git diff --check 65099a5..HEAD
```

Focused GREEN checks also passed:

```text
PASS  structural scanner test — 1 passed, 121 skipped
PASS  event-cache-repair env-unset producer test — outer test and child test passed
```

## Self-review

- The scanner recursively reads only `.rs` files below the three required
  runtime roots and skips `bin`, `build`, `generated`, and `target` path parts.
- It removes Rust test sections before runtime analysis, detects direct env
  checks, env constants, helper functions, boolean aliases, and stderr helper
  calls, and has no production allowlist.
- Synthetic fixtures prove rejection of direct, helper, and boolean-alias
  gated-only recorders; acceptance of collection-before-stderr; and exclusion
  of a test-only environment probe.
- Scanner findings carry a relative path, one-based line, and fixed
  private-data-free reason.
- The only production behavior change is the missing structured collection for
  cache-repair diagnostics. No QA runner output, product state, diagnostics
  bridge contract, or unrelated stderr text changed.
- Commits contain only the intended production/test files; the pre-existing
  Task 3 report was not staged.

## Residual concerns

- `.superpowers/sdd/task-3-report.md` remains pre-existing dirty worktree state
  and is intentionally left untouched.
- The scanner is intentionally conservative text analysis rather than a full
  Rust parser; its direct/helper/alias coverage is locked by synthetic fixtures
  and the clean runtime inventory.
