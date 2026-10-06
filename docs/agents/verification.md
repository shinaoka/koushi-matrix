# Verification Discipline

How correctness is established in this repository. Read this before fixing
anything. The lane catalog is in [qa-lanes.md](qa-lanes.md); the contract
surfaces a change must mirror are in
[state-ownership.md](state-ownership.md#snapshot-and-wire-contract-mirrors).

## Verify first, no human eyes

Correctness is guaranteed by reproducible headless verification, never by manual
or visual GUI inspection. Build the verification setup BEFORE the fix and let
the same check turn green as the proof of the fix: verification first, then the
fix, strictly, never the reverse.

- For any bug / regression / perf / behavior change, FIRST add or extend a
  headless check that REPRODUCES the problem (RED): a `headless-core-qa`
  scenario against a local homeserver, a Rust/TypeScript unit test, or a
  Playwright spec — asserting on `CoreEvent` / `AppStateSnapshot` / tokens /
  DOM, never on logs or fixed sleeps. The fix is "done" only when that same
  check turns GREEN.
- Measure performance claims; never eyeball them. Gate on a number — e.g. the
  `cache_restore` scenario asserts a deep-history anchor is restored from cache
  in ≤ N backward-paginate cycles while the network is blocked.
- To prove cache-served / offline behavior, block the network in-harness (the
  `headless-core-qa` `QaTcpProxy.disable()` pattern) and assert success with no
  `network` origin (the #123 `EventsOrigin` observer).
- Source-text assertions (`source.contains("…")`) are structure guards, not
  behavioral proof. Prefer a test that drives the behavior and asserts on
  emitted events/state.
- Native / manual GUI inspection is the last and weakest layer: a confirmation
  only, never the primary correctness gate.

## Minimize human round trips

Human-in-the-loop debugging is a bottleneck. Always look for ways to minimize
the number of human reproduction and feedback round trips. Rich diagnostics are
one important means: before asking the human to retry, add enough sanitized
information to distinguish the leading hypotheses in one run, including the
relevant stage, outcome, elapsed time, error classification, and useful counts
or booleans. Prefer one deliberately rich diagnostic pass over adding one field
after each retry. Never log secrets, credentials, recovery material, keys,
tokens, or unnecessary raw identifiers.

Before running an expensive Linux/macOS/Windows GUI lane as a debugger, add a
cheap private-data-free diagnostic token or title state for the missing product
transition, then run focused Rust/Tauri/browser checks. Full native GUI lanes
are final evidence for an issue, not the first place to discover command
routing failures.

## Read the gate's own exit status

Read the gate's own exit status, never a pipeline's. `cargo test … | grep …`
reports grep's status, and appending anything (`; echo done`, `; true`) reports
that instead, so a failing suite looks green. Run `<gate> > /tmp/x.log 2>&1;
echo "EXIT=$?"` and report that number.

A subagent's "gates passed" claim is not evidence — re-run the gate yourself.

## Verification by stage

- **Iteration:** start with the smallest reproducing check and expand to the
  affected integration/feature matrix once it passes. The first command need
  not be the entire CI suite.
- **Before review:** read the full diff, including untracked/new files, and run
  checks for the changed contracts. Use the checklist below and the relevant
  state-ownership section; installed generic review skills do not define this
  repository's artifact-update commands or gate matrix.
- **Before merge:** run the local gates in
  [engineering rules](../policies/engineering-rules.md#build-dependencies-qa-gates)
  and inspect required CI results. For documentation-only changes under that
  policy, run `node scripts/check-agents-docs.mjs` when touching this tree,
  check affected links and command references, and run `git diff --check`.
  Do not claim unrun product suites passed.

## Running focused tests

- When running focused Rust crate unit tests, add `--lib` unless integration
  tests are intentionally part of the gate. Example: `cargo test -p koushi-core
  --lib some_unit_test_name`. Without `--lib`, Cargo still launches every
  matching integration-test binary after the library test, which is slow even
  when those binaries run zero tests.
- When running a focused Rust integration test, target the integration-test
  binary with `--test <name>` instead of using only a package-wide name filter.
  Example: use `cargo test -p koushi-state --test search_state`, not `cargo test
  -p koushi-state search`, because the latter launches every integration-test
  binary and then filters inside each one.
- Do not run a long-duration end-to-end or homeserver scenario after every small
  implementation edit. First complete the coherent assertion-driven flow, using
  compile checks, focused unit/integration tests, and short fail-fast
  checkpoints while iterating. Remove superseded fixture paths and review the
  finished diff, then run the long scenario once as the integrated gate. Re-run
  it only when its own evidence identifies a necessary change or after the final
  reviewed fix; do not spend the full timeout to discover one incomplete phase
  at a time.
- When a broad Playwright or browser-headless run reveals multiple failures with
  the same shape, stop one-test-at-a-time spot fixes. First read the shared
  harness, component lifecycle contract, and related fixtures as a group;
  classify whether the problem is fixture drift, a missing DTO mirror, unstable
  Playwright actionability, or product behavior. Repair the shared
  helper/contract boundary before rerunning the broad gate.

## What CI actually gates

`.github/workflows/ci.yml` runs on every pull request. Every job in the table
below, including `Rust lint (rustfmt / clippy)`, is a required status check
for merging into `main`:

| Job | Covers |
| --- | --- |
| `Frontend (typecheck / vitest / build / secret-scan)` | typecheck, vitest, build, secret scan, ESLint import boundaries, Tauri adapter boundary, domain-crate platform deps |
| `Browser headless (Playwright DOM tier)` | `npx playwright test` — a red spec is a blocked merge |
| `Rust lint (rustfmt / clippy)` | `cargo fmt --check`, then workspace, QA-binary, and release-configuration clippy with `-D warnings` (see [Rust lint gate](#rust-lint-gate)) |
| `Rust (workspace / src-tauri / wasm)` | submodule guard, diagnostic-isolation guard, one feature-unified workspace suite (including the `koushi-core-testkit` integration targets and the `koushi-desktop` DTO/IPC contract tests), wasm build, `cargo-deny`, `cargo-machete`, the CI-cache report, and workspace cargo metrics |
| `macOS Tauri cargo check` | `cargo check --profile ci -p koushi-desktop` plus `cargo clippy ... -- -D warnings` on macOS, including `#[cfg(target_os = "macos")]` paths excluded by Linux CI, and runs the native ImageIO decoder tests (`--test image_io_decoder`) |
| `Core invitations (tuwunel)` / `Core invitations (synapse)` | real homeserver `--core --scenario=invites_dm` per server |
| `Core QA binary tests` | `cargo test -p koushi-qa --features qa-bin --bin headless-core-qa` |
| `Windows overlay ACL IPC` | `cargo test -p koushi-windows-overlay-acl windows_overlay_ipc_is_authorized`, OIDC launch tests, and desktop clippy with `-D warnings` for Windows-only cfg paths |

Do not assume a green PR means a homeserver job passed — check the job
explicitly, and confirm whether it is a required check before treating it as a
merge gate.

Do not explain an unusually long CI step as normal repository variance without
comparing it to recent successful runs. Inspect the same job step's duration in
a recent green run; once the current step exceeds twice that baseline, stop
passive waiting and reproduce the exact workflow command locally (including
integration tests and exclusions), or inspect the completed job log if
available.

`cargo test --profile ci --workspace` does not compile the QA binaries: both bin targets set
`required-features = ["qa-bin"]`. Only the `Core QA binary tests` job compiles
them.

CI profile, cache-key, and toolchain-bump maintenance notes are in
[environment.md](environment.md#ci-maintenance).

### Rust lint gate

Run from the repository root with the pinned toolchain (`rust-toolchain.toml`
lists the `clippy` and `rustfmt` components):

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy -p koushi-qa --features qa-bin --all-targets --locked -- -D warnings
cargo clippy --release --workspace --locked -- -D warnings
```

The release run matters because `cfg(debug_assertions)` QA paths disappear
there, so imports that only those paths use are unused only in release builds.

`cargo fmt` formats workspace members only; do not use `cargo fmt --all`,
which also rewrites path dependencies such as `vendor/matrix-rust-sdk`. The
vendored SDK is excluded from the workspace and is not linted.

Fix findings rather than silencing them. Crate- or module-wide `allow`
attributes are not accepted. A per-item `#[expect(lint, reason = "...")]` is
acceptable only when the lint is a false positive or the fix would harm
clarity (for example, a Tauri command whose parameters are named IPC
arguments). Use `#[cfg(target_os = ...)]`, or `cfg(any(<platform>, test))`
for pure helpers that are tested everywhere, for genuinely platform-specific
code. Shared contract variants produced only on one platform keep a narrowly
scoped `cfg_attr(not(<platform>), allow(dead_code))`.

## Diff self-review

Before opening a PR or requesting a review, read the branch's own finished
diff and judge it against the applicable canon yourself. Trace changed
production paths, contract mirrors, async/ownership and terminal semantics;
confirm verify-first evidence and the applicable local gate matrix. The
`preflight-review` skill can supply general prompts, but use this repository's
[artifact instructions](state-ownership.md#snapshot-and-wire-contract-mirrors)
for exact update procedures.

```bash
git diff origin/main...HEAD
git status --short   # untracked files are absent from git diff entirely
```

Review priorities are in
[Review And Audit](../../REPOSITORY_RULES.md#review-and-audit); for the first
one, check `REPOSITORY_RULES.md`, the overview, `state-machine.md` when reducers
change, the engineering rules, `AGENTS.md`, and the relevant dated plan.

User-guide consistency is part of this pre-PR check. Use the
[PR checklist](../../.github/pull_request_template.md) and compare affected
instructions with the implementation and tests: menu/category paths, UI labels,
prerequisites, outcomes, and limitations. Update the guide and
[settings location map](../help/settings.md) in the same PR when behavior or
placement changes; otherwise explain why no manual change is needed. Run
`node scripts/user-help.mjs --check` for navigation and generated llms.txt.
A passing link check alone does not establish that instructions are correct.

For UI changes, check the changed surfaces against
[Property Display And Editing](../../REPOSITORY_RULES.md#property-display-and-editing)
in both self-review and independent audit, and report findings with the
property, both locations, and the effect.

Scope notes that repeatedly matter:

- Read `Cargo.toml` and `src/lib.rs` alongside a change that adds feature gates,
  changes module visibility, or exposes test-only APIs. Judging the change
  without them invents problems that are not there.
- Include new files explicitly. `git diff` alone is empty for untracked paths,
  so a review that only reads it can miss an entire new module.
- When a finding is caused by a canon gap rather than this change, amend the
  canon too — see the rule-update requirement in `REPOSITORY_RULES.md`.
- Check new guards and fallbacks against engineering rules "Design Simplicity".

## Flake probe

`.github/workflows/issue-738-flake-probe.yml` is a scheduled/manual,
non-required measurement job; the required CI workflow remains retry-free, and a
failed probe cannot turn a required check green. It runs a closed list of named
probes at one SHA with a 120-second bound per attempt, compiles Rust test
binaries in a warm-up step outside measured attempts, and records only fixed
failure signatures. A workflow rerun must not replace or hide failed attempt
records. Run locally with a bounded attempt count:

```bash
node scripts/flake-probe.mjs --attempts 10 --output-dir artifacts/issue-738-flake-probe
node scripts/summarize-flake-probe.mjs --sha <40-hex-sha> artifacts/issue-738-flake-probe/flake-probe-results.json
```

The summarizer accepts one or more result artifacts, reports attempt totals and
failure rate over the observed date window, validates one unchanged SHA with
`--sha <40-hex-sha>` or `--require-unchanged-sha`, and exits nonzero at or above
the `--max-failure-rate` threshold (for example `0.01`).

## IME-safe text input checks

When changing any text field, textarea, password/recovery entry, upload caption,
search box, or form, use the primitives in
`apps/desktop/src/components/ImeTextControl.tsx`. Run the focused contract and
the production surface inventory from the repository root:

```bash
node --test scripts/check-ime-text-inputs.test.mjs
node scripts/check-ime-text-inputs.mjs
npm --prefix apps/desktop test -- src/components/ImeTextControl.test.tsx
```

The normal desktop lint command (`npm --prefix apps/desktop run lint`) includes
the inventory gate. If the gate finds a new surface, migrate it to the shared
primitive. Do not add a per-file exception or local composition workaround.

## Cost-controlled agent delegation

- Use cheaper implementation agents only for bounded, low-ambiguity work: source
  search, issue inventory, single-file tests, small module-local Rust patches,
  docs consistency checks, and narrow diff reviews. Prompts must name the issue,
  allowed files, forbidden shared files, expected verification command, and the
  exact output format.
- Ownership and hot-file limits follow
  [Parallel Implementation Protocol](../../REPOSITORY_RULES.md#parallel-implementation-protocol)
  and [Shared Hot Files](../../REPOSITORY_RULES.md#shared-hot-files); canon docs,
  commits, issue comments, and close decisions also stay with the main agent.
- Review prompts name the applicable canon sections and follow
  [Review And Audit](../../REPOSITORY_RULES.md#review-and-audit). A silent,
  timed-out, or budget-exceeded run is not review evidence. Higher-priority
  agent instructions may add review requirements; repository guidance does
  not cancel them.
