# #840: read generation without copying account state

Status: implemented and reviewed; required CI pending.

## Boundary

`CoreConnection::versioned_snapshot()` clones the watched AppState even when a
caller immediately discards everything except `generation`. Add
`state_generation()` to copy that same scalar under the same watch borrow.
Migrate the 19 generation-only production calls in Core navigation/media staging
and Tauri session/timeline commands, plus their test usages. Keep full-state reads
where callers actually use state; their removal belongs to the scoped migration.

This is a localized accessor/caller change, not a new ownership design or state
transition. No command, event, DTO, generation rule, request correlation, deadline,
account fence, composer lease or snapshot publication behavior changes. Therefore
it does not require a new consequential-design review/canon amendment. The parent
implements it and obtains one read-only cross-model final review after local gates.

## Verification and limits

- Strengthened the existing real command-admission test: the scalar API must agree
  with the published snapshot generation, while preserving the existing composer
  mode and admission assertions. Compile-RED was the missing public method;
  subsequent command-admission tests passed. This is API/behavior evidence, not
  an allocation benchmark or the separate 1,500-reader/publication RED turning green.
- The getter body borrows `.generation` directly; it does not clone the snapshot
  or account state. No new allocation profiler, unsafe code or dependency is needed
  to inspect this one-expression data path.
- Core 944 passed / 8 existing ignored; Core testkit 225 passed; Tauri 127 passed;
  SDK 143 passed; State 786 plus its doctest passed; the new Core public-item
  doctest passed. No existing behavioral test item was removed.
- Formatting, SDK/domain/Tauri/protocol boundaries, test structure, docs and diff
  checks passed. No frontend code or wire shape changed; Tauri wire/IPC contract
  tests passed. Parent reviewed the full diff before requesting final review.
- SDK/Tauri dependency compilation was separate setup, not test execution time.
- Logs: `/tmp/issue840-generation-{red,green,core,testkit,tauri,doc}.log`.

The full scoped-publication contract and its independent read-receipt/profile
regressions remain ongoing #839/#840/#846 work. This change does not remove the
whole-state watch, per-event projection copies, full outcome snapshots or frontend
snapshot recovery and does not claim epic completion.

Final review: DeepSeek V4 Flash, read-only, low — **Correct-to-merge subject to
CI**, no findings. Reviewed complete patch SHA-256
`4b08bedeebdaa48b852409d368e6bedf956de4aa43dd5ed692f38e6abf8c779e` and confirmed
all 19 production replacements, unchanged watch semantics and retained assertions.
One final round; no re-review required.
