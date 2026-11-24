# Reject `scroll_back_cached_history` without `--core` (2026-11-24, #2079)

`scripts/desktop-headless-local-qa.mjs` admitted `--scenario=scroll_back_cached_history`
without `--core` and exited 0 without running anything: the SDK lane skips this scenario
(one of eight it skips, which is a smaller set than the core-only guards), and the Core
lane only runs under `if (runCoreQa)`. The nightly lane was unaffected because it always
passes `--core`; the false green hit manual and local invocations.

Canon: `docs/agents/qa-lanes.md` (`scroll_back_cached_history` — "Requires `--core`,
separate from `all`", and its six tokens) and `REPOSITORY_RULES.md` (QA gate changes
leave a worklog). The doc line was already correct and is unchanged.

## Change

- Admission guard next to the other core-only scenarios, same shape and wording:
  `--scenario=scroll_back_cached_history requires --core because it validates Core state`.
- `thread_receipt_fresh_session` gets the same arm: `docs/agents/qa-lanes.md` already
  documents it as core-only, and without the guard the runner started the generic SDK
  suite, never started Core, and exited 0 — the identical false green, one scenario
  over. It is *not* added to the SDK-skip condition: that list answers "is the fixed SDK
  suite irrelevant here" (`read_state_convergence` is guarded and still runs it), while
  the guards answer "does this stage need Core".
- `scripts/lib/qa-token-contract.mjs`: the six tokens `scroll_back_cached_history`
  requires are registered, plus the five for `thread_receipt_fresh_session`, so both
  names now have a contract entry instead of the empty fallback
  (`requiredTokensForHeadlessScenario` returned `[]`). What this buys is a drift pin, not
  a checkpoint check: the Core binary prints `scenario_report`, whose body is the stage's
  *declared* token list (`crates/koushi-qa/src/bin/headless_core_qa/registry.rs`,
  `scenario_report`), and the runner scans that same stdout, so a scenario that skipped a
  checkpoint print would still satisfy `assertRequiredTokens`. Making the assertion able
  to fail on a skipped checkpoint needs the report to stop echoing the expectation, and
  that is a change to every scenario's output — left as a follow-up.
- `assertSdkSubmoduleSynced` moved from the first statement of `run()` to just after
  argument admission (after the `--fixture-run` and `--cargo-profile` checks). Admission
  now fails on the arguments alone instead of being masked by an environment check, and
  the paths that fail at admission stay runnable without a git submodule or a toolchain.
- `all` needed no change: the scenario is absent from `COVERED_BY_ALL`
  (`scripts/lib/headless-qa-lanes.mjs`) and excluded from `QaStage::All` in
  `crates/koushi-qa/src/bin/headless_core_qa/registry.rs`.

## Verification

- `node scripts/desktop-headless-local-qa.mjs --run --scenario=scroll_back_cached_history`
  — before: prints `headless SDK QA skipped for core-only scenario
  scroll_back_cached_history` and exits **0**; after: `headless local QA failed:
  --scenario=scroll_back_cached_history requires --core because it validates Core
  state`, exit **1**, no homeserver started. Same before/after for
  `thread_receipt_fresh_session`.
- `apps/desktop/src/scripts/qaTokenContract.test.ts` (one case per scenario): the
  required tokens in order, each checked against the scenario's own
  `QaStage::... => &[...]` arm in `registry.rs`, so a Rust-side rename cannot leave the
  lane permanently red. The arm is matched by regex rather than sliced between two
  neighbouring names, so reordering the arms stays green and a renamed neighbour cannot
  widen the window. Teeth checked by renaming a token in `registry.rs` and watching the
  case go red. RED before the contract change (`expected [] to deeply equal [...]`).
- `apps/desktop/src/scripts/headlessAndRealQa.test.ts` (two new cases, one per
  scenario): the `--run --scenario=...` spawn fails at admission and never prints the
  SDK-skip line (RED before the guard: the spawn exited 0); with `--core` the scenario
  passes admission and stops at the later `--fixture-run` guard, which proves admission
  let it through without starting a lane.
- `node scripts/check-headless-qa-lanes.mjs` — `headless QA lanes ok (47 scenarios)`.
- `node --test scripts/lib/*.test.mjs` — 32 passed, 1 skipped.
- `npm run typecheck`, `npm run lint`, `npm run test -- --run` in `apps/desktop`.

## Not covered

`printUsage`'s scenario list and the `--list` `checks` array are hand-maintained
enumerations that already omit several registered scenarios (the `checks` array lists 41
of 47, including neither `scroll_back_cached_history` nor
`thread_receipt_fresh_session`); this fix does not extend either, because they are not
admission and adding one name to one of two incomplete lists hides the gap rather than
closing it.

Two more instances of the same class were found while fixing this and left as
follow-ups rather than folded in:

- `encryption_debug` is in the SDK-skip condition and the usage string but has no
  `QaStage` arm, so `--run --scenario=encryption_debug` prints the skip line and exits 0
  while `--core` rejects the name. Deleting the skip arm alone would keep it green (the
  generic SDK suite would run), so the fix is rejecting unknown scenario names at
  admission — a different change.
- The 15 hand-written `--core` guards (14 with the identical `requires --core because it
  validates Core state` wording plus `history_key_recovery_window`), the SDK-skip
  condition, `NIGHTLY_SCENARIOS` and `COVERED_BY_ALL` are separate classifications of the
  same scenario set; their drift is what produced this issue. One exported
  `CORE_ONLY_SCENARIOS` in `scripts/lib/headless-qa-lanes.mjs` driving the guards is the
  durable fix for admission. The SDK-skip condition stays a separate, smaller set — as
  above, it answers "is the fixed SDK suite irrelevant here", and seven guarded scenarios
  still run it — so the shared list must not be wired into it without deciding each name.
  That consolidation deserves its own diff and review.

Review record: two independent rounds by `ollama-cloud/deepseek-v4.1-flash`
(`/tmp/2079-review/issue2079.out`, `issue2079b.out`).

Round 1 — BLOCKER: the spawn tests would fail in both vitest CI jobs, which check out no
submodules and would hit `assertSdkSubmoduleSynced` before the guards; fixed at the runner
by moving that assertion below admission. IMPORTANT: `thread_receipt_fresh_session` is
documented core-only and was unguarded — fixed. IMPORTANT: `encryption_debug` is a dead
name that exits 0 without running anything — deferred above, since deleting its skip arm
alone keeps it green. Round 2 — IMPORTANT: the registered tokens cannot detect a skipped
checkpoint because the binary echoes the declared list onto the same stdout; wording here
corrected, real fix deferred above. MINORs applied: dead assertion removed, `--list`
claims corrected, `--server` and `--fixture-run` argument checks moved above the
environment check, registry pin bounded by regex, `thread_receipt_fresh_session` tokens
registered, this section corrected.
