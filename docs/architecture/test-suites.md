# Rust integration test suites

Status: normative. Suite layout and correctness-gate invocation for #1710.
Crate ownership remains in [overview.md](overview.md); test placement policy is
in [engineering rules](../policies/engineering-rules.md#test-placement).

## Layout and ownership

Integration tests remain feature-sized Rust modules, grouped into Cargo suite
binaries with `tests/<suite>/main.rs` containing `mod <feature>;` declarations.
A new case goes in its existing feature module; a new feature gets a sibling
module and a declaration in that suite's entry point, not a new top-level
`tests/*.rs` binary. Nested feature modules move with their parent. Suite roots
contain wiring and crate-level attributes only, never test bodies.

| Crate | Suite | Owner / new-test placement |
| --- | --- | --- |
| `koushi-state` | `state` | All public reducer, state, preference and projection contracts |
| `koushi-sdk` | `sdk` | SDK wrapper contracts and local mock-server adapters |
| `koushi-core` | `core` | Existing public Core leaf contracts; private unit-test modules stay attached to the library |
| `koushi-core-testkit` | `runtime` | Existing `runtime_*` modules except the process-isolated targets below; new runtime/actor interaction contracts |
| `koushi-core-testkit` | `contracts` | Other shared Core integration contracts (login, media, request outcomes, wire DTOs, subscription residency) |
| `koushi-core-testkit` | `stack_budget` (standalone) | Existing small-worker-stack process-abort proof, also run in release mode on Linux/Windows |
| `koushi-core-testkit` | `runtime_navigation` (standalone) | Exact process-global navigation persistence counts; other runtime tests emit the same diagnostic events without the reader lock |
| `koushi-mcp-host` | `bridge` | Host bridge, admission, consent and attachment contracts; retain each module's platform cfg |

Other crates' small existing integration targets remain unchanged in this
migration. Do not create another suite simply for one feature; add a suite only
when an independently required configuration or process boundary justifies it.
The standalone targets keep their existing command names and guards. In
`runtime_navigation::scroll_anchor_updates_coalesce_into_one_navigation_persist`,
the diagnostic-reader lock does not lock other runtimes' writers, so grouping
that target would invalidate its exact global event-count oracle. Preserve that
process boundary rather than add locks to unrelated cases.

Shared support stays in `tests/support/`. Each former test root keeps its own
support-module instance via a relative path declaration: this preserves helper
state scope AND every compiled helper test instance. In particular, the OAuth
fixture currently contributes two tests to each Core testkit module that imports
support; consolidating these repeated cases would change coverage inventory and
is not part of this migration. The existing exported test-only helper macro
must become module-scoped when several former roots share a suite, with local
imports rather than duplicate `macro_export` definitions. No product API changes.
Crate-only recursion-limit attributes move to the entry point (maximum of the
former roots); module cfg attributes remain on their original feature module.

## Process, concurrency and resources

Libtest runs cases concurrently within each suite using its normal thread
policy; Cargo runs test targets separately. Grouping does not impose ordering,
`--test-threads=1`, new serial annotations, or wider sleeps/timeouts. The migration
must keep each assertion, case, cfg and ignore attribute unchanged. Existing
ignored cases remain opt-in; do not introduce ignores to get a suite green.

State tests construct their own state; Core tests keep isolated runtime/store
and credential directories, and mock servers keep dynamically allocated local
ports. No suite-level homeserver, runtime or store singleton is introduced.
Existing diagnostic ring readers retain the shared diagnostic lock. Existing
module-local locks remain module-local. Process-wide environment, current
working directory and global tracing/diagnostic state are shared by all cases:
new tests must not mutate environment/cwd or clear global state concurrently.
Tests requiring real process isolation must retain a documented separate target
rather than disabling parallelism for an entire suite.

Automated homeserver QA still uses the existing disposable Tuwunel/Synapse
runners; these suites use their existing local fixtures and never real accounts.
If consolidation exposes cross-module interference requiring extensive serial
annotations or changes to test intent, stop and report alternatives before
continuing. GUI inspection cannot replace the headless suite evidence.

## One correctness-gate configuration

The canonical complete local run, from the repository root with the pinned
Rust toolchain and no custom `RUSTFLAGS`/`CARGO_ENCODED_RUSTFLAGS`, is:

```bash
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=12 KACHE_EXPLAIN_MISS=1 cargo test --workspace --profile ci --locked
```

The hosted Rust job runs that same feature-unified suite with its existing
`--exclude sidebar-composition --exclude key-management` (non-product spikes).
Keep one workspace invocation in that job, including Core testkit and desktop.
Do not follow it with per-package proofs of cases already executed, including
`oauth_authorization_code`: the workspace suite already executes those cases,
and a second `-p koushi-core-testkit` run resolves a smaller dependency feature
graph and relinks the SDK stack and its test binary with a different hash.
Earlier separate Core testkit/desktop runs had the same problem. Cargo retains
old hashes; directory totals include these stale executables until removed.
This is not evidence that libtest ran each case twice in one invocation.

Focused iteration remains allowed and is deliberately not an artifact-size
baseline: `cargo test -p <crate> --profile ci --test <suite> <feature>::` compiles
only the owning suite but may select another dependency feature graph. Do not
normalize those graphs by enabling speculative production features. QA binaries
(`qa-bin`), wasm, release stack budgets and platform jobs remain distinct gates:
they test genuinely different configurations. No wrapper or second config source
is needed. Profile/feature/toolchain/flags changes legitimately produce new hashes.

## Relinking cost and verification

A suite statically links the SDK once instead of once per feature module. Using
the issue's roughly 150 MB SDK-linked executable as an estimate, 40 testkit
integration roots become two suites plus two process-isolated targets:
roughly 6 GB becomes 600 MB for one configuration, before actual platform/code
size differences. State is SDK-free and benefits mainly from fewer rustc/link
invocations. SDK and bridge each pay one SDK link per configuration. Any edit in
a suite recompiles and relinks that whole suite, even for a filtered case;
`runtime` versus `contracts` bounds this cost without a new binary per feature.
Edits to shared support can relink both suites and both standalone targets.
These are size/link-count estimates, not measured wall-time promises.

Before moving files, capture Cargo metadata, per-crate `--profile ci --no-run`
artifact paths, sizes, and each executable's libtest `--list` output. Compare the
complete case multiset afterwards, removing only the added feature-module
prefix (including repeated support cases); a matching total alone is insufficient.
Preserve cfg/ignore status and verify wiring-only diffs. Record per-crate totals,
current executable paths, `target/ci/deps` total bytes, executable bytes and count
above 100,000,000 bytes. Retire only precisely inventoried obsolete test artifacts
inside this checkout after recording them; do not delete shared caches or other
worktrees. Record cleanup separately from savings in the current artifact set.
A headless structure check must reject new per-feature Cargo roots in migrated
crates and undeclared feature modules in each suite, and require one workspace
test invocation in the hosted Rust job. Finish with one full workspace test run,
checking doctests as well as libtest results, and the source/architecture guards
whose paths moved. Measurements and
review verdicts belong in the task work record and PR, not this durable policy.
