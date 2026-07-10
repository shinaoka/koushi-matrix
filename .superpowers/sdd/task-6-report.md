# Task 6 — Always-on unified diagnostics

## Status

Complete after the final scanner closure hardening. The scanner rejects the
reviewed semantic-association, multiline-gate, negative-early-exit,
comment/string fabrication, batch-record, control-flow implication, loop
pairing, transitive helper/alias, and cross-file-helper escape paths.
It accepts the current always-on runtime collectors, including the batched
timeline collectors and the cross-file unread collector. The runtime inventory
returns no findings.

## Commits

- `9d5c526` — `fix: complete core diagnostic capture`
- `d6c0f37` — `feat: batch structured diagnostic records`
- `da8b82a` — `fix: harden diagnostics scanner parsing`
- `a394316` — `docs: record final diagnostics scanner verification`
- `6f63d8d` — `fix: close diagnostics scanner control-flow gaps`
- This report — `docs: record scanner closure evidence`

The scanner commits change only
`apps/desktop/src/scripts/releaseScripts.test.ts`. The report commits change
only this report. The pre-existing dirty
`.superpowers/sdd/task-3-report.md` remained untouched and retained blob hash
`24115f232aaafa4fe8795f9c946197e32ddef3e3` throughout this work.

## Final scanner behavior

The scanner now:

1. Lexes Rust comments, normal strings, raw strings, and nested block comments
   into a line-preserving code view before record/helper discovery. Diagnostic
   literal and format-placeholder tokens are extracted separately.
2. Associates mirrors only through actual `record`, `record_batch`, structured
   helper, `eprintln!`, and stderr-helper arguments, plus referenced local
   bindings, loop bindings, and event-vector `push`/`extend` data flow. Gate
   helper names and unrelated bridge statements do not contribute tokens.
3. Applies conditional control-flow barriers before accepting a collector.
   Normalized conjunctive gate conditions must imply the collector condition
   with the same polarity. Opposite-polarity and disjunctive gates are rejected;
   the existing paired `dropped_queries > 0` runtime form remains accepted.
4. Recognizes balanced multiline environment conditions and negative
   early-return gates whose sibling stderr/helper calls are controlled by the
   return.
5. Recognizes generic `record(make_diagnostic_event(stage))`, wrapper helpers,
   arbitrary local event bindings, `record_batch(events)`, and vector-built
   events.
6. Treats collector loops as barriers unless the gated mirror contains a paired
   iterator over the same source data. Existing timeline and unread loops remain
   accepted while an independent post-loop trace gate is rejected.
7. Resolves environment, structured, and stderr helpers to a fixed point across
   scanned files. Arbitrary alias-to-alias chains, two-hop `Self` record
   wrappers, and normalized `crate::`/`self::`/`super::` qualification are
   recognized without a file/function allowlist.
8. Uses the generic private-data-free finding reason
   `env-gated diagnostic producer has no always-on structured collection`.

The source-line-preserving `cfg(test)` masking also stops at cfg-gated struct
fields, so a removed test field cannot consume the enclosing production brace.
Balanced one-line functions and gates also end their own scopes, preventing
duplicate findings from overlapping scopes.

## TDD evidence

### RED

Before scanner implementation, the initial adversarial command covering
semantic association, generic linked records, multiline/negative gates,
lexical fabrication, generic gated records, and the existing baseline returned
exit 1: `6 failed | 127 skipped (133 total)`.

Observed failures were:

- all three semantic-association probes returned no findings;
- all three valid generic pre-gate collectors returned findings;
- all three multiline/negative bad probes returned no findings;
- the block-comment and string-helper probes fabricated collectors;
- the generic reason assertions still received the stderr-specific reason.

The separate cross-file fixture returned exit 1:
`1 failed | 133 skipped (134 total)` because the module-qualified runtime gate
returned no finding.

The batch/vector fixture was added before batch recognition and returned exit
1: `1 failed | 134 skipped (135 total)` because a valid always-on
`record_batch(diagnostic_events)` before a negative early exit was reported as
missing collection.

The final closure fixtures were then added before implementation. Their focused
command returned exit 1: `3 failed | 135 skipped (138 total)`:

- the negative-polarity, disjunctive, and independent-loop cases all returned
  no findings;
- the wrapped environment helper, arbitrary alias chain, crate-qualified
  cross-file helper, and two-hop `Self` record wrapper all returned no findings;
- the balanced one-line fixture returned two duplicate findings instead of one.

### GREEN

Focused command covering the baseline, gated record-only producers, canonical
unrelated records, semantic barriers, generic records/wrappers/bindings,
record batches, transitive stderr, multiline and negative gates, cross-file
helpers, condition polarity/conjunction implication, paired and independent
loops, arbitrary/transitive aliases, normalized qualification, two-hop `Self`
wrappers, balanced one-line scopes, lexical sanitization,
direct/helper/loop/transformed mirrors, cfg handling, line preservation, and
runtime inventory:

```text
PASS  21 passed | 121 skipped (142 total)
```

Full release-script file:

```text
PASS  npm --prefix apps/desktop test -- src/scripts/releaseScripts.test.ts
      142 passed (142)
```

## Exact inventory

Command:

```bash
rg -n "KOUSHI_[A-Z0-9_]*(TRACE|DIAGNOST)|VITE_KOUSHI_VERBOSE_DIAGNOSTICS" \
  crates/koushi-sdk/src crates/koushi-core/src apps/desktop/src-tauri/src apps/desktop/src \
  --glob '!**/bin/**'
```

Result: exit 0, 88 matches. Classification totals:

1. Stderr mirror gates with collection first — 15 matches.
2. Test-only environment/compatibility assertions and synthetic scanner
   fixtures — 58 matches.
3. Comments, constants, or helpers consumed only by category 1 — 14 matches.
4. The removed Vite-variable assertion — 1 match.

The totals are `15 + 58 + 14 + 1 = 88`. The runtime scanner assertion returned
an empty finding list.

## Production gaps

Cross-file discovery exposed the `raw_room_list_trace` path gated by
`unread_trace::enabled()`. Commit `9d5c526` moved capture before the optional
stderr gate and added an environment-unset real-reducer regression. The scanner
fixture preserves the exact cross-file alias/helper shape so reintroducing the
gate fails structurally.

Commit `d6c0f37` introduced batched timeline diagnostic recording. The scanner
recognizes `record_batch(events)` and follows event-vector bindings before the
negative early exits at the timeline item/diff mirrors.

## Verification

All requested commands returned exit status 0:

```text
PASS  cargo fmt --all -- --check
PASS  room_list_applied_records_through_real_reducer_with_trace_env_unset
      outer 1 passed + env-unset child 1 passed; 423 filtered in each process
PASS  event_cache_repair_diagnostic_runs_without_trace_environment
      outer 1 passed + env-unset child 1 passed; 423 filtered in each process
PASS  focused releaseScripts scanner suite — 21 passed, 121 skipped
PASS  full releaseScripts.test.ts — 142 passed
PASS  npm --prefix apps/desktop test — 739 passed across 46 files
PASS  npm --prefix apps/desktop run typecheck
PASS  npm --prefix apps/desktop run lint
PASS  npm --prefix apps/desktop run lint:tauri-boundary
PASS  npm --prefix apps/desktop run lint:domain-deps
PASS  npm --prefix apps/desktop run qa:secret-scan
PASS  npm --prefix apps/desktop run qa:release-gates -- --no-compile
PASS  exact inventory — 88 matches, classified 15/58/14/1
PASS  git diff --check 65099a5
PASS  git diff --check
PASS  staged scanner scope — releaseScripts.test.ts only
```

## Self-review

- No runtime allowlist, per-file exception, or line window was added.
- No debug print, timing probe, `TODO`, or `FIXME` remains in the scanner diff.
- Gate/helper identifiers and unrelated statements cannot fabricate semantic
  association.
- Comments and strings cannot fabricate record or helper discovery.
- Current direct, helper, paired-loop, transformed, batch, early-exit,
  polarity/conjunction, transitive alias/helper, normalized qualification,
  cross-file, one-line, cfg, line-preservation, and runtime-inventory forms are
  covered.
- Each scanner commit contains one file; each report commit contains one file;
  Task 3 remains unstaged and unchanged.

## Residual concerns

- The scanner is deliberately conservative text analysis rather than a full
  Rust parser. Its supported lexical, control-flow, local-data-flow, and helper
  forms are locked by adversarial fixtures and the live runtime inventory.
- Cross-file helper qualification is derived from Rust source module filenames
  (and parent directory names for `mod.rs`). A future renamed re-export may
  require an additional generic resolution form.
- Boolean implication is intentionally limited to conjunctions of normalized
  atoms. More complex equivalent expressions are conservatively rejected.
