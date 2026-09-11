# Thread root refresh flash

The main timeline could replace an accepted thread root with `Loading thread
message…` and a reply count while refreshing its aggregate. The service retained
the root item, but `display_data()` exposed aggregate operation status as root
hydration status, causing the Rust display projection to select a placeholder.

The service now projects pending/failed placeholder status only without an
accepted root item. Internal refresh/failure tracking is preserved. Both
canonical roots and hydrated off-window roots retain their bodies through
refresh and failure, and successful refresh still updates the reply count.

Canon consulted: `REPOSITORY_RULES.md`, `docs/agents/verification.md`, and the
thread-root projection lifecycle in `docs/architecture/state-machine.md`. The
latter now explicitly documents the existing Ready-to-Ready display contract.

Changed implementation: `crates/koushi-core/src/threads_list.rs`. Regression
coverage lives in the new `timeline/display_projection/thread_refresh_tests.rs`
module, registered by `timeline/display_projection.rs`.

Verification:

- Before the fix, `cargo test -p koushi-core --lib thread_refresh` failed with
  `ThreadRootPending` instead of `ThreadRoot`; the missing-root control passed.
- After the fix, the same command passed both tests, including canonical and
  hydrated roots, pending refresh, failure, successful refresh, and missing root.
- `cargo test -p koushi-core --lib thread`: 86 passed.
- `cargo test -p koushi-core --lib timeline::display_projection`: 24 passed.
- SDK submodule guard, Rust test structure checker, and `git diff --check` passed.

Self-review confirmed that the change affects display DTO status only, preserves
the authoritative service's operation state, and adds no UI-owned state or waits.
No native app build or installation was performed.
