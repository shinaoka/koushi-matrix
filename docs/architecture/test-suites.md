# Rust integration test suites

Status: normative. Owns integration-test binary layout and execution policy
(Refs #1710); [Test Placement](../policies/engineering-rules.md#test-placement)
owns the unit/integration boundary. Product crate ownership is unchanged.

## Layout and ownership

Use Cargo's standard `tests/<suite>/main.rs` entry point with explicit
`mod <feature>;` declarations. Each existing feature file becomes a module,
not a merged test file. The owning crate continues to own its cases and fixtures.
New cases go in the matching feature module; new features add a module and its
`mod` declaration to the existing suite. Do not create a binary per feature.

| Crate | Suite / membership |
| --- | --- |
| `koushi-state` | `contracts`: all current integration targets, including the existing `navigation_state` and `session_state` submodules |
| `koushi-sdk` | `contracts`: all current integration targets |
| `koushi-core` | `contracts`: all current Cargo integration targets; `tests/timeline_mutations/` remains a private source-test module, not a new integration target |
| `koushi-core-testkit` | `runtime`: `runtime_*` except `runtime_navigation`; `contracts`: all other current targets except `stack_budget`; retain standalone `runtime_navigation` and `stack_budget` |
| `koushi-mcp-host`, `koushi-mcp`, `koushi-key`, `koushi-search`, `koushi-media` | One `contracts` suite per crate containing its current integration targets |
| `koushi-desktop` | `contracts`: `legacy_sso_callback`, `window_zoom_hotkeys`; retain platform-gated `image_io_decoder` and live-keyring `keyring_backend` as standalone targets |
| Other workspace members | No integration targets to move; retain unit, binary, example and doc tests |

The declarations in each `main.rs` are the exhaustive membership list. Move
feature-local subdirectories with their owner; preserve fixture bytes and adjust
only relative fixture/module paths. Shared support stays at `tests/support/`.
Existing support inclusions stay feature-local: the OAuth fixture currently
contributes two tests to each of 27 testkit targets that includes it (54 fixture
case instances). Preserve every one of those registered cases, not just the two
distinct function bodies. Executable inventories, not this source count, prove
case preservation.
Make the testkit helper macro module-scoped rather than exporting the same macro
multiple times into a combined crate; preserve its expansion and helper calls.

## Process, parallelism and isolation

- Suites use the normal libtest runner, not a custom harness. Preserve test
  functions, assertions, attributes, platform/feature guards, per-test Tokio
  runtime configuration and `#[ignore]` reasons. Adding a module-name prefix to
  a registered test name is the only intended identity change.
- Each test retains its own runtime, temporary directories, credential-store
  fixture and mock/disposable homeserver. Do not introduce suite-wide sessions,
  stores, environment mutation or server reuse. Socket fixtures bind ephemeral
  loopback ports. No external homeserver or real account is used.
- Keep libtest's existing parallel scheduling and `RUST_TEST_THREADS` override;
  do not globally serialize suites or add `#[serial]` to make aggregation pass.
  The build budget remains `CARGO_BUILD_JOBS=10` or less; Cargo build jobs and
  libtest threads are separate controls. Record any test-thread override with
  validation evidence and use the same override before and after.
- Process globals now span more feature modules. Preserve local locks and
  owner-scoped diagnostic probes. `runtime_navigation` stays isolated because
  its space-order diagnostic assertion observes process-wide persistence counts;
  `stack_budget` stays directly addressable for its release worker-stack gate.
  Live credential-store and macOS decoder targets keep their existing platform
  gate entry points. Ignored live-keyring tests remain ignored; aggregation does
  not authorize running them.
- If further cross-feature interference appears, stop and report it with options;
  do not add mass serialization, change assertions, or silently drop cases.

## One ordinary CI configuration

The canonical ordinary Rust gate is the existing feature-unified invocation:

```bash
KACHE_EXPLAIN_MISS=1 cargo test --profile ci --workspace --exclude sidebar-composition --exclude key-management
```

Local acceptance for this migration additionally runs the literal whole workspace
(`cargo test --workspace --profile ci`, with `KACHE_EXPLAIN_MISS=1` and the same
build budget), including the two spike members. Do not change profile settings,
SDK features, `RUSTFLAGS`, or `CARGO_ENCODED_RUSTFLAGS` between inventory and run.

Do not follow the workspace gate with a per-package build of cases it already
ran. In particular, CI's separate `-p koushi-core-testkit --test
oauth_authorization_code` step repeats cases already included in the workspace
and resolves a different dependency feature graph. Remove that redundant step;
OAuth proof remains in `contracts::oauth_authorization_code` with its full cases.
Workspace selection unifies dev-dependency features from all selected members;
a focused `-p` selection does not. With the same toolchain, lockfile and no
rustflags overrides, `cargo tree --workspace -e normal,build,dev` enables Tokio's
`io-std`, `process`, `signal` and `signal-hook-registry` features, while
`cargo tree -p koushi-core-testkit -e normal,build,dev` does not. The SDK's own
feature names match but it links a differently configured Tokio dependency.
Thus the redundant OAuth step can rebuild the transitive stack even without a
Matrix SDK feature or rustflags change. This identifies a current reproducible
configuration split, not the provenance of every historical artifact in #1710.
Distinct artifact hashes alone are not proof of a rustflags difference or two
active gates. Old hashes are stale build output,
not additional test coverage. Do not remove features or weaken the workspace gate
to make hashes match.

QA-binary `qa-bin` tests, wasm, platform checks, and the release `stack_budget`
gate remain separate because they cover different targets, features or profiles.
For an intentional focused iteration use `--test <suite> <feature>::` and report
that it is not the canonical workspace gate; it may resolve a second graph.
Disposable homeserver QA retains `--server=tuwunel|synapse|both` and its existing
runner contract.

## Linking trade-off

At the issue's approximate 150 MiB per SDK-linked test binary, changing one
feature now recompiles/relinks its containing suite once, not a tiny individual
binary. Touching several files in one suite still pays one link, whereas changing
a shared dependency relinks every affected suite. The testkit's 40 targets become
four and the SDK's 14 become one; the state crate's 93 targets become one (it does
not itself link the SDK). Most remaining crates have one suite. A combined binary
may be larger than 150 MiB; this is a disk/link-count estimate, not a claim that
one-suite iteration becomes faster. Do not introduce extra suite splits until
measured iteration cost or an actual process-isolation need justifies them.

## Migration evidence

Before changing layout, capture Cargo's executable inventory and each executable's
`--list --format terse` output, including ignored tests. Afterward compare the
per-crate multisets of registered cases after removing only the added suite
prefix; include zero-test targets, unit/binary tests and unchanged doc-test
coverage separately. No assertion, case, or repeated fixture case may disappear.
Check ignored status as well as counts. Report per-crate before/after totals and
the final executable paths grouped by crate/suite in the PR.

Measure `target/ci/deps` in bytes (both total directory allocation and executable
logical bytes), counting regular ELF executables over 100 MiB separately from
shared libraries and build outputs. State whether the baseline is a fresh build
or a previously mixed tree; never substitute the issue's historical measurement
for a measurement in this checkout. Remove only retired artifacts in this
checkout after recording their exact paths; keep dependencies and unrelated
artifacts. Record removed bytes separately so cleanup is not misrepresented as
suite-only savings. Do not touch another worktree or a shared target directory.
