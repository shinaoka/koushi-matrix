# Issue #947 CI latency implementation

Issue #947 identified three avoidable sources of Rust CI latency: the cache
action restored `target` while Cargo wrote to job-specific `CARGO_TARGET_DIR`
paths, the Rust job reran package suites already covered by the workspace
suite, and hosted builds used the default debuginfo-heavy test profile.

This implementation keeps the distinct QA gates while making the ordinary
Rust path explicit:

- Rust cache mappings now match `target-ci`, `target-macos-check`,
  `target-windows-overlay`, and the non-required Issue #738 probe target.
- The workspace job excludes `koushi-core-testkit` and `koushi-desktop`, then
  runs each package once through an explicit `cargo test -p ... --profile ci`
  step. This satisfies the leaf-crate contract and preserves the Tauri DTO/IPC
  gate without compiling either package twice.
- `[profile.ci]` inherits `test`, keeps debug assertions and overflow checks,
  and disables debuginfo, incremental compilation, and symbols for hosted
  correctness CI. Local development and release profiles are unchanged.
- The primary Rust job caches representative vendored Matrix SDK artifacts
  with keys including runner, Rust toolchain, profile, enabled SDK feature set,
  workspace dependency inputs, and the checked-out SDK revision.
  `ci-rust-cache-report.mjs` records cache-hit state, target size, fingerprints,
  and SDK artifacts, and fails if a claimed SDK hit is empty.
- The headless QA helper accepts `--cargo-profile=ci`, and CI homeserver,
  macOS, Windows, and Rust commands select the profile explicitly.
- `.github/workflows/issue-947-ci-benchmark.yml` provides a manual cold/warm
  comparison for one immutable source and SDK revision. It captures Cargo
  compilation/checking/finish counts, test totals, cache-hit state, target and
  log sizes, and elapsed workspace-suite time as downloadable JSON and step
  summaries.

The issue's baseline was a median 16m15s Rust-job duration across 14 recent
successful runs. That baseline and the arithmetic estimate in Issue #947 are
kept as pre-change measurements; this change does not claim a post-change
speedup until a cold cache population run and a compatible warm run are
available. The CI cache report provides the measurements needed to compare
target paths, restored artifacts, target size, and later compilation behavior.
