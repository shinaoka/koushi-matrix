# Desktop test code without `env!("CARGO_MANIFEST_DIR")` (#1937)

`AGENTS.md:37-41` forbids `env!("CARGO_MANIFEST_DIR")` in test code: `env!` expands
the absolute checkout path at compile time and rustc records it as
`env-dep:CARGO_MANIFEST_DIR=<absolute path>` in dep-info, so kache keys the test unit
on the checkout location and rebuilds it in every worktree. #1919 fixed the same
pattern under `crates/`; this is the desktop crate.

## The six sites, classified

| Site | Use | Was | Now |
| --- | --- | --- | --- |
| `src/tests.rs:18` | compile-time | `include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/capabilities/windows-overlay.json"))` | `include_str!("../capabilities/windows-overlay.json")` |
| `src/core_event_forwarder/tests.rs:2001` | compile-time | same pattern for `/../src/domain/coreEvents.generated.json` | `include_str!("../../../src/domain/coreEvents.generated.json")` |
| `tests/image_io_decoder.rs:33` | compile-time | `include_bytes!(concat!(env!(…), "/../../../crates/koushi-media/tests/fixtures/heif/opaque.heic"))` | `include_bytes!("../../../../crates/koushi-media/tests/fixtures/heif/opaque.heic")` |
| `src/core_event_forwarder/tests.rs:1889` | **run-time** | `concat!(…)` used by `fs::write` and `fs::read_to_string` | `Path::new(&var_os("CARGO_MANIFEST_DIR")…).join("../src/domain/coreEvents.generated.json")` |
| `src/dto/tests.rs:2620` | **run-time** | `concat!(…)` for `tests/golden/frontend_app_state.json` (`UPDATE_GOLDEN` write + read) | `Path::new(&var_os(…)).join("tests/golden/frontend_app_state.json")`, `display()` in the panic message |
| `tests/window_zoom_hotkeys.rs:11` | **run-time** | `Path::new(env!("CARGO_MANIFEST_DIR")).join(file)` | `Path::new(&var_os(…)).join(file)` |

The issue's summary lists the `dto/tests.rs` site and the `core_event_forwarder`
`contract_path` alongside the compile-time ones; both are run-time (they feed
`std::fs`), so they take the `std::env::var_os` form, not a source-relative include.
No product code changed.

Note the `include_str!` base moves with the macro's own file, not with the manifest
directory: `src/tests.rs` reaches `capabilities/` with `..`, the module-dir test files
need `../../..` to get back to `apps/desktop/src/domain/`, and the integration test in
`tests/` needs `../../../../` for the repository root. The three paths were each
verified by compiling, not by arithmetic (`cargo check --all-targets` fails loudly on a
wrong one).

## Evidence: dep-info before and after

`KACHE_EXPLAIN_MISS=1 cargo test -p koushi-desktop --lib --no-run`, then the dep-info
file rustc wrote for the lib test unit (`target/debug/deps/koushi_desktop-b812fad2eeebc5e9.d`
— the same file name before and after, so the pair is directly comparable):

| | `env-dep` lines | `env-dep:CARGO_MANIFEST_DIR` | absolute paths from the checkout |
| --- | --- | --- | --- |
| `origin/main` (`41bef7a2`) | 7 | 1 (`…/koushi-1937/apps/desktop/src-tauri`) | source + `target/` |
| this branch | 6 | **0** | only `target/debug/build/…` (`OUT_DIR`) and the output binary |

The six that remain are `CARGO_PKG_AUTHORS`, `CARGO_PKG_DESCRIPTION`,
`CARGO_PKG_NAME`, `CARGO_PKG_VERSION`, `KOUSHI_UPDATER_PUBLIC_KEY` and `OUT_DIR`.
They come from product code (`env!("CARGO_PKG_*")`, `env!("OUT_DIR")`) and from cargo's
own build metadata, not from the test tree; `cargo test --no-run` still emits them, so
the task's literal "`grep -c env-dep` is 0" is not reachable without touching product
code. What matters for kache is the checkout path: `CARGO_MANIFEST_DIR` is gone from
every `koushi_desktop-*.d`, and the source dependencies listed there are relative
(`apps/desktop/src-tauri/src/…`) instead of absolute. `OUT_DIR` still carries the
target directory's absolute path, which is product-code territory — recorded here as
the next candidate if cross-checkout reuse for this crate still misses.

After `cargo test -p koushi-desktop --no-run` (all targets, integration tests
included): `grep -c 'env-dep:CARGO_MANIFEST_DIR' target/debug/deps/koushi_desktop-*.d`
is 0 for all seven files.

Not measured: a second worktree's kache hit. It needs a full desktop test build in a
fresh worktree, and the dep-info above already shows the only checkout-dependent key
input is gone.

## Gates

- `cargo fmt --check` (workspace members; **not** `cargo fmt --all`, which the repo
  forbids — `.github/workflows/ci.yml:626`, `docs/agents/verification.md:176` — and
  which also reformats `vendor/matrix-rust-sdk`).
- `cargo test -p koushi-desktop --lib`: 322 passed, 1 ignored.
- `cargo test -p koushi-desktop --test image_io_decoder --test window_zoom_hotkeys`:
  9 passed (the two integration tests that read fixtures).
- `cargo clippy --profile ci -p koushi-desktop --all-targets --locked -- -D warnings`:
  no issues.
- `node scripts/check-rust-test-structure.mjs` (clean) and `node --test
  scripts/check-rust-test-structure.test.mjs` (24 pass).

## Rule consistency

`AGENTS.md:37-41` already states the rule without a crate scope and prescribes exactly
these two forms ("`include_bytes!`/`include_str!` with a path relative to the source
file" and "`std::env::var_os("CARGO_MANIFEST_DIR")`"), so no documentation change was
needed.

The gap the issue names — that `scripts/check-rust-test-structure.mjs` resolves
`env!("CARGO_MANIFEST_DIR")` instead of rejecting it, which is why CI stayed green — is
still open. Bounding a repo-wide rule needs a definition of "test code" for the checker
and is not part of this change; it is the follow-up that would keep this from
returning.
