# Matrix SDK Submodule Path Guard Design

## Problem

The repository carries its Matrix SDK fork in `vendor/matrix-rust-sdk`, but the
root workspace previously resolved Matrix SDK crates from a pinned Git URL.
Local submodule changes could therefore pass their own tests while the desktop
application silently compiled unrelated, older source. This made real fixes
appear ineffective and invalidated integration testing.

## Invariant

The checked-in submodule gitlink is the only Matrix SDK revision pin. Every
root workspace Matrix SDK dependency must resolve through the corresponding
path below `vendor/matrix-rust-sdk`; root Git URL or `rev` declarations for
these crates are forbidden.

The guarded dependencies are:

- `matrix-sdk`
- `matrix-sdk-base`
- `matrix-sdk-search`
- `matrix-sdk-test`
- `matrix-sdk-ui`

## Mechanical Enforcement

Refactor the existing SDK submodule guard to verify two independent facts:

1. `Cargo.toml` declares every guarded dependency exactly once with its
   expected submodule path and contains no `git` or `rev` key for it.
2. `git submodule status` reports `vendor/matrix-rust-sdk` initialized and at
   the revision recorded by the current superproject gitlink.

The guard must fail closed for missing, duplicate, wrong-path, Git-backed, or
mixed declarations. Diagnostics must name the violated invariant without
printing repository-private values or revisions.

Existing callers continue to use `assertSdkSubmoduleSynced`; its implementation
changes from comparing two separately pinned revisions to validating the path
contract plus the gitlink checkout state.

## Documentation

Add the durable rule to `REPOSITORY_RULES.md`. Add a concise operational note
and the verification commands to `AGENTS.md`; it must link back to the durable
rule rather than becoming a second normative definition.

## Tests

Use fixture manifests and submodule-status fixtures. Add tests before changing
the implementation and observe them fail for the missing path-contract API.
The final focused gate covers:

- the repository's current five path dependencies are accepted;
- a Git URL plus `rev` declaration is rejected;
- a missing or wrong path is rejected;
- mixed path/Git declarations are rejected;
- uninitialized, stale, conflicted, and missing submodule states remain
  rejected;
- the command-line guard emits a private-data-free failure.

Run the focused Node test and the production guard. The existing long-running
desktop QA suites are not required because this change is a static build-input
contract.

## Non-goals

- The guard does not require the submodule worktree to be clean; local SDK
  development must remain possible.
- It does not duplicate Cargo's dependency resolver.
- It does not manage or update the submodule revision.
- It does not inspect nested third-party dependency sources outside the five
  workspace declarations.
