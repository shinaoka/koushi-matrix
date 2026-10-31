# Issue #1659: retire the browser/wasm requirement

## Scope and canon-first sequence

The maintainer withdrew the browser-hosted target on 2026-10-11 and confirmed
that decision on 2026-10-29; [#1659](https://github.com/shinaoka/koushi-matrix/issues/1659)
is the source for this retirement.

Consulted `REPOSITORY_RULES.md` (Canon-First Change Protocol, Architecture And
Ownership, Review And Audit), overview "Platform Portability", engineering
rules "Build, Dependencies, QA Gates", and the environment/verification guides.
A completed GPT-6.1 Sol pre-review accepted the plan with one documentation
addition: update the maintained integration-edge-case gate inventory too.
The overview and engineering rules were amended before enforcement changed.
The root rules contained no browser/wasm requirement and needed no edit.

Transport-neutral DTOs, executor/actor ownership, native platform ports, the
fail-closed encryption model, and shared Rust semantics remain required for
native hosts, headless QA, testability, and layering. Browser DOM tests still
cover the desktop React view; they are not a browser-hosted Rust build.

## Enforcement changes and retained code inventory

- `ci.yml`: remove only the wasm leaf compile step and wasm target installation.
- `issue-947-ci-benchmark.yml`: remove wasm installation from both toolchain steps.
- `rust-toolchain.toml`: stop automatically installing the wasm target.
- Desktop `package.json`: remove `qa:wasm-check`; no lockfile/dependency changes.
- Update environment, verification, test-suite, and integration-edge-case gate
  descriptions. Reword the domain-dependency guard's wasm rationale without
  changing its dependency bans or scanning behavior.
- Replace the positive wasm toolchain assertion with a focused structure test
  covering the toolchain, npm entry, workflows, and stable required check name.

Removed jobs: **none**. The branch-protection API still requires
`Rust (workspace / src-tauri / wasm)`, so that historical check name is unchanged.
The only removed executable step is `wasm build check (state + search + protocol)`.
It and the removed npm entry invoke Cargo directly; neither leaves an orphaned
script. All other job definitions, steps, and npm scripts are unchanged.

Retained, without runtime/source changes:

- `crates/koushi-core/src/executor.rs`: native Tokio task/timer wrapper,
  including `AbortOnDrop`; retain for lifecycle ownership and headless testing.
  Its comment about a possible wasm backend is aspirational, not a supported
  target or a reason to change the wrapper in this task.
- `crates/koushi-sdk/src/e2ee.rs` and `src/lib.rs`: four
  `cfg(not(target_family = "wasm"))` declarations for file-based room-key
  export/import and their re-exports; unchanged.
- `crates/koushi-sdk/src/room_operations.rs`: wasm-compatible UUID comment;
  no special implementation to remove. Vendored SDK code is untouched.

## Verification

All work used this Mac and this worktree; no remote execution or homeserver QA.

- `kache doctor --verify`: 32,749 valid entries, zero corruption.
- Pinned SDK submodule initialized; `node scripts/check-sdk-submodule.mjs` passed.
- `KACHE_EXPLAIN_MISS=1 cargo metadata --locked --format-version 1`: passed
  (18 workspace members, 841 resolved packages); lockfile unchanged.
- `node scripts/check-agents-docs.mjs`: passed.
- Focused browser-retirement structure check: RED before removing enforcement,
  GREEN afterwards (`node --test --test-name-pattern='browser wasm'
  scripts/build-structure-contract.test.mjs`).
- Domain-dependency checker tests: 2 passed. Domain dependency, protocol/QA,
  leaf-boundary, and headless-lane guards all passed.
- PyYAML parsed both changed workflows. A semantic comparison against the base
  commit, allowing only the enumerated wasm removals and toolchain-step rename,
  passed: all 12 main CI jobs and both benchmark jobs retained, all other
  workflow fields and steps identical. Equivalent comparisons confirmed no
  other npm-script or toolchain changes.
- Full build-structure/domain check: 9 passed, 2 failed. The original
  build-structure test, run with base-commit file contents, has the same two
  failures: its dev-profile test expects `debug = "line-tables-only"` while
  `Cargo.toml` uses `debug = 0`; its invitation test expects an inline
  `--scenario=invites_dm` while CI delegates to `headless-qa-lane.mjs --lane=pr`.
  These unrelated failures are not changed or presented as green here.
- Existing documentation/code drift: environment instructions also describe
  line-table dev debuginfo, unlike the current `Cargo.toml`; this unrelated
  profile discrepancy is left for a separate change.

No user-visible product behavior changes, so help pages, screenshots, and
`llms.txt` need no update. Rust compilation, full product tests, native GUI,
release builds, and homeserver QA are not claimed by this configuration-only
change. Historical verification records are preserved.
