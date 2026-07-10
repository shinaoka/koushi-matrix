# Task 6 — Always-on unified diagnostics

## Status

Complete for the final Terra scanner correction. The scanner now rejects both
unrelated canonical pre-gate records and both generic gated record-only
producers, while accepting the current runtime mirrors and transitive stderr
helpers. No production Rust source or runtime behavior changed.

## Commits

- `08f60e0` — `fix: collect event cache repair diagnostics without trace gates`
- `c0b0b45` — `test: enforce always-on diagnostic collection`
- `0f068b8` — `fix: harden diagnostics gate inventory`
- `7ba84ab` — `docs: update Task 6 fix evidence`
- `96c78c7` — `fix: close diagnostics scanner escape paths`
- `c7d1d25` — `fix: distinguish diagnostic mirrors from unrelated records`
- This report — `docs: finalize diagnostics scanner evidence`

The source/tests commit changes only:

- `apps/desktop/src/scripts/releaseScripts.test.ts`

The report commit changes only this report. The pre-existing dirty
`.superpowers/sdd/task-3-report.md` was not edited or staged.

## Final scanner correction

The scanner now keeps two separate contracts:

1. Generic `record(...)` recognition drives gated-producer detection and the
   transitive structured-helper closure. This catches direct
   `record(make_diagnostic_event())` and helpers containing that form, even
   when no stderr call exists.
2. Pre-gate mirror acceptance uses canonical structured producers only, walks
   enclosing sibling statements, and requires semantic association with the
   gated mirror. Association uses shared identifiers, fixed diagnostic tokens,
   format captures, local event bindings, and statement bridges; it does not
   use a line window, production allowlist, or file/function exception.

The positive fixtures cover direct, helper, boolean-alias, loop,
post-record-transformation, and two-hop stderr-helper mirrors. Existing nested
`cfg` parsing and source-line preservation remain covered.

## TDD evidence

### RED

After adding the canonical unrelated direct/helper probes and the generic
direct/helper record-only probes, before the scanner correction:

```bash
npm --prefix apps/desktop test -- src/scripts/releaseScripts.test.ts -t "always-on diagnostic collection rejects trace-only producers and accepts stderr mirrors|scanner does not let an unrelated record hide a later gated-only diagnostic|scanner recognizes generic gated record producers without stderr|scanner accepts direct, helper, loop, and transformed mirror siblings|stderr helper discovery follows two-hop chains without masking gated-only output"
```

Result: exit 1; `2 failed | 3 passed | 124 skipped (129 total)`. The canonical
unrelated and generic record-only fixtures each returned zero findings before
the fix; the existing bad/good, two-hop, and mirror-shape positives passed.

### GREEN

The same focused command after implementation: exit 0; `5 passed | 124
skipped (129 total)`.

The focused release-script file after implementation:

```bash
npm --prefix apps/desktop test -- src/scripts/releaseScripts.test.ts
```

Result: exit 0; `129 passed (129)`.

## Exact inventory

Command:

```bash
rg -n "KOUSHI_[A-Z0-9_]*(TRACE|DIAGNOST)|VITE_KOUSHI_VERBOSE_DIAGNOSTICS" \
  crates/koushi-sdk/src crates/koushi-core/src apps/desktop/src-tauri/src apps/desktop/src \
  --glob '!**/bin/**'
```

Result: exit 0, 68 matches. Classification totals:

1. Stderr mirror gates with collection first — 15 matches:
   `apps/desktop/src-tauri/src/commands/mod.rs` (117, 141, 696),
   `crates/koushi-core/src/room.rs` (2745), `account.rs` (1078, 1079, 4946,
   4961), `timeline.rs` (316, 885, 956, 3110, 5429), and `runtime.rs` (125,
   1139).
2. Test-only environment/compatibility assertions, including synthetic
   scanner fixtures — 38 matches. Current scanner-fixture matches are at
   `releaseScripts.test.ts` lines 761, 771, 779, 786, 800, 815, 866, 883,
   888, 1011, 1100, 1108, 1116, 1124, 1147, and 1154; the remaining matches
   are the existing Tauri/core env-unset, source-assertion, and compatibility
   tests.
3. Comments, constants, or helpers consumed by category 1 — 14 matches:
   `commands/search.rs:6`, `commands/mod.rs:691`, `koushi-sdk/src/lib.rs:55`,
   `search.rs:75`, `unread_trace.rs:10`, `sync.rs:75`, `account.rs:89`,
   `timeline.rs:881,1566,1784`, `startup_trace.rs:4,44`, and
   `runtime.rs:80,109`.
4. Task 5 removed-Vite-variable assertion — 1 match at
   `apps/desktop/src/App.diagnostics.test.tsx:199`.

The totals are 15 + 38 + 14 + 1 = 68. The runtime-source assertion returned
an empty finding list.

## Production gaps

No new production gap was exposed. The earlier event-cache-repair producer
remains the only production fix: it records a typed, private-data-free event
before its unchanged optional stderr mirror. This final wave changed only the
scanner tests and did not change product state, latency instrumentation,
diagnostics transport, privacy boundaries, or stderr text.

## Verification

All requested gates returned exit status 0:

```text
PASS  cargo fmt --all -- --check
PASS  CARGO_TARGET_DIR=/Users/hiroshi/projects/Element-dev/matrix-desktop/target cargo test -p koushi-core --lib event_cache_repair_diagnostic_runs_without_trace_environment — outer and env-unset child tests passed (1 + 1)
PASS  CARGO_TARGET_DIR=/Users/hiroshi/projects/Element-dev/matrix-desktop/target cargo test -p koushi-diagnostics --lib — 8 passed
PASS  CARGO_TARGET_DIR=/Users/hiroshi/projects/Element-dev/matrix-desktop/target cargo test -p koushi-sdk --lib — 43 passed
PASS  CARGO_TARGET_DIR=/Users/hiroshi/projects/Element-dev/matrix-desktop/target cargo test -p koushi-core --lib — 417 passed, 2 ignored
PASS  CARGO_TARGET_DIR=/Users/hiroshi/projects/Element-dev/matrix-desktop/target cargo test -p koushi-desktop — 96 passed, 1 ignored; integration 5 passed
PASS  npm --prefix apps/desktop test -- src/scripts/releaseScripts.test.ts — 129 passed
PASS  npm --prefix apps/desktop test — 722 passed across 46 files
PASS  npm --prefix apps/desktop run typecheck
PASS  npm --prefix apps/desktop run lint
PASS  npm --prefix apps/desktop run lint:tauri-boundary
PASS  npm --prefix apps/desktop run lint:domain-deps
PASS  npm --prefix apps/desktop run qa:secret-scan
PASS  npm --prefix apps/desktop run qa:release-gates -- --no-compile
PASS  exact inventory command — 68 matches
PASS  git diff --check 65099a5..HEAD
PASS  git diff --check
PASS  staged source/tests scope check — releaseScripts.test.ts only
```

## Self-review

- No production allowlist or runtime source change was added.
- Generic gated-producer recognition is separate from strict mirror
  association, and both canonical unrelated probes are required to fail.
- Direct, helper, alias, loop, transformed, transitive-helper, nested-`cfg`,
  line-preservation, and runtime inventory paths are covered.
- The source/tests commit contains one file; the report commit contains one
  report; Task 3 remains untouched.

## Residual concerns

- The scanner remains conservative text analysis rather than a full Rust
  parser. Its structural sibling and semantic association forms are locked by
  the runtime scan and synthetic adversarial fixtures.
- `.superpowers/sdd/task-3-report.md` remains pre-existing dirty worktree state
  and is intentionally left untouched.
