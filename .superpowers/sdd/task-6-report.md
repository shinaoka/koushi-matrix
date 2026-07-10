# Task 6 — Always-on unified diagnostics

## Status

Complete for the second Terra fix wave. The scanner now rejects the three
reported escape paths, accepts the current runtime collection/mirror forms,
and reports no findings across the runtime Rust roots. No production runtime
code or diagnostic behavior changed in this wave.

## Commits

- `08f60e0` — `fix: collect event cache repair diagnostics without trace gates`
- `c0b0b45` — `test: enforce always-on diagnostic collection`
- `0f068b8` — `fix: harden diagnostics gate inventory`
- `7ba84ab` — `docs: update Task 6 fix evidence`
- `96c78c7` — `fix: close diagnostics scanner escape paths`
- This report — `docs: record diagnostics scanner fix evidence`

The source/tests commit changes only:

- `apps/desktop/src/scripts/releaseScripts.test.ts`

The pre-existing dirty `.superpowers/sdd/task-3-report.md` was not edited or
staged.

## Terra findings fixed

1. Pre-gate collection is associated through preceding structural sibling
   statements in the same enclosing block, stopping at another record or a
   control-flow boundary. The arbitrary 64-line lexical window is gone. This
   retains current direct, helper, loop, and post-record transformation mirror
   forms while an unrelated `record(unrelated_event())` does not satisfy the
   gate.
2. `stderrHelpers` now computes the transitive local helper-call closure, so a
   gated two-hop stderr helper is detected. A matching always-on collection
   mirror remains accepted.
3. Test masking parses nested `cfg(all(...))`/`cfg(any(...))` expressions. It
   masks exact `test` and expressions provably requiring `test`; it keeps
   `cfg(any(test, feature = "diagnostic-runtime"))` in the scan. Source lines
   remain in place, preserving finding line numbers.

## TDD evidence

### RED

After adding the three regression fixture groups and before implementing the
scanner changes:

```bash
npm --prefix apps/desktop test -- src/scripts/releaseScripts.test.ts -t "scanner does not let an unrelated record hide a later gated-only diagnostic|stderr helper discovery follows two-hop chains without masking gated-only output|scanner masks only cfg items that are provably test-only"
```

Result: exit 1; `3 failed | 124 skipped | 127 total`. The unrelated-record
and two-hop fixtures each received zero findings where one was required. The
cfg fixture reported the wrong remaining item (`line 28` instead of the
conditional runtime gate), proving the masking regression was exercised.

### GREEN

After implementation, the same focused command returned exit 0:

```text
Test Files  1 passed (1)
Tests       3 passed | 124 skipped (127)
```

The complete release-script file then returned exit 0:

```bash
npm --prefix apps/desktop test -- src/scripts/releaseScripts.test.ts
```

```text
Test Files  1 passed (1)
Tests       127 passed (127)
```

## Exact inventory

Command:

```bash
rg -n "KOUSHI_[A-Z0-9_]*(TRACE|DIAGNOST)|VITE_KOUSHI_VERBOSE_DIAGNOSTICS" \
  crates/koushi-sdk/src crates/koushi-core/src apps/desktop/src-tauri/src apps/desktop/src \
  --glob '!**/bin/**'
```

Result: exit 0, 68 matches. Classification totals are:

1. Stderr mirror gates with collection first — 15 matches:
   `apps/desktop/src-tauri/src/commands/mod.rs` (117, 141, 696),
   `crates/koushi-core/src/room.rs` (2745), `account.rs` (1078, 1079, 4946,
   4961), `timeline.rs` (316, 885, 956, 3110, 5429), and `runtime.rs` (125,
   1139).
2. Test-only environment/compatibility assertions, including synthetic
   scanner fixtures — 38 matches. This includes the Tauri/core test env
   removal/assertion lines, the release-script fixtures at lines 600, 609,
   617, 624, 638, 688, 705, 710, 741, 769, 801, 809, 817, 825, 848, and
   855, and its existing QA assertions at 1613, 1625, and 2036.
3. Comments, constants, or helpers consumed by category 1 — 14 matches:
   `commands/search.rs:6`, `commands/mod.rs:691`, `koushi-sdk/src/lib.rs:55`,
   `search.rs:75`, `unread_trace.rs:10`, `sync.rs:75`, `account.rs:89`,
   `timeline.rs:881,1566,1784`, `startup_trace.rs:4,44`, and
   `runtime.rs:80,109`.
4. Task 5 removed-Vite-variable assertion — 1 match at
   `apps/desktop/src/App.diagnostics.test.tsx:199`.

The totals are 15 + 38 + 14 + 1 = 68. The scanner's runtime-source assertion
returned an empty finding list.

## Production gaps

No new production gap was exposed by Terra's three scanner findings. The
previous `AccountActor::handle_ensure_room_event_cached` repair remains the
only production fix: it records `core.event_cache_repair` with a typed request
ID and fixed stage/outcome/reason tokens before the unchanged optional stderr
mirror. The actor env-unset test remains green, and no product state, latency
instrumentation, privacy boundary, or stderr text changed in this wave.

## Verification

All requested commands returned exit status 0:

```text
PASS  cargo fmt --all -- --check
PASS  CARGO_TARGET_DIR=/Users/hiroshi/projects/Element-dev/matrix-desktop/target cargo test -p koushi-core --lib event_cache_repair_diagnostic_runs_without_trace_environment — outer and env-unset child tests passed (1 + 1)
PASS  npm --prefix apps/desktop test -- src/scripts/releaseScripts.test.ts — 127 passed
PASS  npm --prefix apps/desktop run typecheck
PASS  npm --prefix apps/desktop run lint
PASS  npm --prefix apps/desktop run lint:tauri-boundary
PASS  npm --prefix apps/desktop run lint:domain-deps
PASS  npm --prefix apps/desktop run qa:secret-scan
PASS  npm --prefix apps/desktop run qa:release-gates -- --no-compile
PASS  exact inventory command — 68 matches
PASS  git diff --check
PASS  git diff --cached --check before source/tests commit
```

The focused RED/GREEN commands and full release-script test were run after the
source/test edit and before the evidence-report edit. The source/tests commit
was independently checked for staged scope and contains one file only.

## Self-review

- No production allowlist was added; scanner behavior is syntax/structure
  driven and covered by positive and negative synthetic fixtures.
- Direct, helper, boolean-alias, transitive stderr-helper, nested cfg, and
  balanced test-item paths are covered without logging private data.
- The scanner still reads only the required runtime roots and skips bin,
  build, generated, and target path components.
- The report commit is separate from the source/tests commit, and Task 3
  remains untouched.

## Residual concerns

- The scanner is intentionally conservative text analysis rather than a full
  Rust parser; its accepted structural forms are locked by the runtime scan
  and the synthetic adversarial fixtures.
- `.superpowers/sdd/task-3-report.md` remains pre-existing dirty worktree
  state and is intentionally left untouched.
