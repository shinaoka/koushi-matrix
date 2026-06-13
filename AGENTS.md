# Agent Notes

This file is operational troubleshooting for agents and QA automation in this
environment. The binding rules distilled from these notes (prohibitions,
secret handling, automation rules, gates) live in
[docs/policies/engineering-rules.md](docs/policies/engineering-rules.md), and
the long-term architecture in
[docs/architecture/overview.md](docs/architecture/overview.md). When a note
here hardens into a durable rule, promote it to the policies document and keep
the operational detail here.

## Implementation Working Rules

All agents implementing the headless core runtime follow
[docs/superpowers/plans/2026-06-12-headless-core-runtime-implementation.md](docs/superpowers/plans/2026-06-12-headless-core-runtime-implementation.md).
All agents implementing the Phase 10+ product surface follow
[docs/superpowers/plans/2026-06-13-phase-10-ui-headless-product-surface.md](docs/superpowers/plans/2026-06-13-phase-10-ui-headless-product-surface.md).

- **Headless-first, local-server-first.** New Matrix behavior lands in
  `matrix-desktop-core`, verified via `CoreCommand`/`CoreEvent` against local
  Conduit/Tuwunel QA, before any Tauri/React wiring. GUI-first implementation
  is prohibited.
- **SDK fork management.** Upstream SDK deltas live on the
  `github.com/shinaoka/matrix-rust-sdk-work` submodule branch
  (`shinaoka/search-ngram`). Local code comments should explain the patch
  surfaces, and
  `docs/upstream/matrix-rust-sdk-feedback.md` stays the place for PR
  candidates. Edit vendored SDK code only inside that submodule branch, then
  update the superproject submodule pointer intentionally.
- **SDK adapter naming.** The low-level Matrix SDK adapter crate is
  `matrix-desktop-sdk`. It owns SDK-facing primitives only; app state,
  actor lifecycle, and QA orchestration stay in `matrix-desktop-core`.
- **Canon-first redesign protocol.** Implementation will hit gaps the design
  did not foresee. When code contradicts the canon or the canon is silent:
  stop coding on that point — do not improvise an undocumented behavior.
  Record what was assumed vs. what was observed. Amend
  `docs/architecture/overview.md` first (and
  `docs/policies/engineering-rules.md` if a rule changes; bump
  `Last amended`), sync the dated spec if the public API changes, add a
  Changelog entry to the implementation plan, and only then implement to the
  amended design. Code that diverges from the canon must not land.
- **Canon amendments always escalate.** The implementing model never amends
  the canon itself. When a design gap requires changing
  `docs/architecture/overview.md` or `docs/policies/engineering-rules.md`,
  stop and hand the redesign decision to the strongest available model of
  the agent's family — for Claude agents Fable 5 or Opus, for Codex agents
  the highest GPT version (never a mini/lightweight tier) — or to the user.
  The implementing model resumes only after the canon is amended. See Model
  Assignment in the implementation plan.
- **Phase exits include a docs-sync check**: no known contradiction between
  landed code and the canon documents.
- **Phase 10+ GUI launch policy.** React UI, DOM scroll behavior, command
  shapes, fake `CoreEvent` streams, and Tauri IPC mock behavior are verified
  in headless browser tests. Do not launch the native Tauri app for these.
  Native GUI smoke is reserved for real IPC, native window, OS menu, WebView,
  and keychain/system-dialog behavior; on macOS it is attended only.

## Local Gates Setup

- Enable the repo pre-commit hook once per clone:
  `git config core.hooksPath .githooks`. It runs the secret scan on staged
  files (`scripts/desktop-secret-scan.mjs --staged`).
- Gate commands (from `apps/desktop`): `npm run qa:secret-scan`,
  `npm run qa:wasm-check` (requires
  `rustup target add wasm32-unknown-unknown`), `npm run qa:release-gates`
  (structural credential-gate check plus `cargo check --release`; the compile
  step is slow on a cold target dir — use
  `node ../../scripts/desktop-release-gate-check.mjs --no-compile` for the
  quick structural pass).
- There is no hosted CI in this repo yet; these gates run locally and in
  `release:preflight`. Wire them into CI when CI infrastructure appears.

## macOS GUI Smoke Failures

- `npm --prefix apps/desktop run qa:mac-gui` controls the Tauri window through
  macOS `System Events`. If it fails with `AppleScript timed out while
  controlling System Events`, grant Accessibility permission to the app running
  the agent, such as Codex, Terminal, or iTerm, then restart that app.
- If Accessibility is already enabled but the same timeout repeats, check
  Privacy & Security > Automation and allow the same app to control
  `System Events`. Restart the agent app after changing either permission.
- A repeated timeout can also be caused by AppleScript code, not permissions.
  In this repo, `process <variable>` hung when resolving the Tauri process.
  Use `first process whose name is <variable>` for variable process names.
- If screenshot capture is blocked, also grant Screen Recording permission to
  the app running the agent.
- In Tauri dev mode the macOS process name can be `matrix-desktop-app`, while
  the product/window title is `matrix-desktop`. GUI automation must check both
  names.
- Failed GUI smoke runs must clean up the full process group. A stale Vite
  process leaves port `5173` occupied and makes the next `tauri dev` fail.
- If a GUI smoke run is interrupted manually with Ctrl-C, verify that
  `lsof -nP -iTCP:5173 -sTCP:LISTEN` is empty before retrying. A stale
  `npm run tauri dev` process group can survive interruption and make the next
  run fail before the app reads the QA login FIFO.
- Do not pass the parent shell environment wholesale into GUI smoke child
  processes. Filter out secret-like variables such as API keys, tokens, and
  passwords before spawning `npm run tauri dev`.
- First-run GUI smoke should set `MATRIX_DESKTOP_SKIP_SAVED_SESSIONS=1`.
  Otherwise opening User Settings can read the macOS Keychain and show a
  confirmation prompt, which blocks unattended automation.
- Do not use `Cmd+Q` to stop the Tauri app from GUI smoke. If focus slips, the
  shortcut can reach Codex and trigger the "Quit Codex?" confirmation dialog.
  Let the script's process-group cleanup stop `tauri dev` and the app instead.

## Real Account Smoke Failures

- If `password-login-smoke --real-account-qa` fails at sync but
  `--check-room-list` succeeds, isolate the restore path first. A no-store
  `restore_session` can diverge from the product path; real-account QA should
  restore with a temporary encrypted SQLite SDK store, cache path, and encrypted
  search index path.
- The smoke CLI must try logout cleanup after any post-login QA failure unless
  `--keep-session` was explicitly requested. Otherwise failed sync/timeline QA
  can leave a live smoke device on the homeserver.
- `qa:real-homeserver` writes `qa.log` synchronously before leak checks and
  exit handling. If the log is missing after a fast successful exit, treat it
  as a regression in the runner.
- Store-backed Matrix SDK sessions must be dropped while a Tokio runtime context
  is entered. Dropping a sqlite-backed SDK client after the runtime context is
  gone can panic in `deadpool-runtime` with `there is no reactor running`.
- In this environment, starting `qa:mac-gui -- --real-login-from-stdin` through a
  non-interactive `exec_command` can deliver immediate stdin EOF. Use a PTY with
  terminal echo disabled, such as `stty -echo; npm --prefix apps/desktop run
  qa:mac-gui -- --real-login-from-stdin; exit_code=$?; stty echo; exit $exit_code`,
  then send the credential lines through stdin.
- Do not drive real-account login by fixed window-relative coordinates. A
  2026-06-12 GUI smoke attempt clicked the wrong login field and placed the
  password in the username field. Real-login GUI smoke should pass credentials
  through `MATRIX_DESKTOP_QA_LOGIN_PIPE`, which contains only a FIFO path in the
  environment and keeps the credential payload out of argv, logs, screenshots,
  and committed files.
- Real-login GUI smoke must set `MATRIX_DESKTOP_SKIP_KEYCHAIN_PERSISTENCE=1`.
  `MATRIX_DESKTOP_SKIP_SAVED_SESSIONS=1` only prevents saved-session reads; a
  successful login can still prompt macOS Keychain during session persistence or
  encrypted SDK store key creation.
- The `password-login-smoke` prompt order is homeserver, username, device name,
  then password. The `qa:mac-gui -- --real-login-from-stdin` order is
  homeserver, username, password, device name, then optional recovery code.
  Leave the fifth line empty to accept `needsRecovery` as a post-login sync QA
  state; provide it only when verifying recovery completion to `ready`.
- When driving `qa:mac-gui -- --real-login-from-stdin` through a PTY, send all
  five newline-terminated lines. Without the fifth blank or recovery line, the
  reader waits for more input and the Tauri window is never launched.
- Do not store post-login real-account screenshots. They can contain room names,
  Matrix IDs, message bodies, or attachment names. Real-account GUI automation
  should rely on private-data-free QA window-title tokens instead. Use
  `--allow-private-screenshots` only for explicitly approved test accounts whose
  post-login room and message data may be written to ignored artifacts.
- Some sparse QA accounts have valid room-list sync but no visible timeline
  items in the automatically selected room. Keep the strict
  `timeline_items > 0` release signal for normal real-account smoke, but use
  `qa:mac-gui -- --allow-empty-timeline` for sparse test accounts when the goal
  is validating login, room-list sync, and GUI panel automation.
- Avoid repeated destructive real-account login cycles while debugging GUI
  automation. Prefer preserving the same running Tauri session while iterating
  on panel/menu checks, and only restart when the script or Tauri capability
  changes require it.
- Use `qa:mac-gui -- --qa-profile=<name>` when a real-account GUI run should
  preserve SDK SQLite store, cache, search index, saved session, and incremental
  sync state across runs. Profile names must be synthetic and non-secret; data is
  stored under ignored `.local-secrets/qa-profiles/<name>/data`.
- The default `qa:mac-gui -- --real-login-from-stdin` path is intentionally
  disposable and sets `MATRIX_DESKTOP_SKIP_KEYCHAIN_PERSISTENCE=1`.
  `--qa-profile=<name>` is the opt-in path for persistent restore/sync QA and
  must set `MATRIX_DESKTOP_QA_FILE_CREDENTIAL_STORE_DIR` so unattended runs do
  not prompt macOS Keychain. This env-controlled file credential store must stay
  behind a debug/test-only compile-time gate; release builds must ignore it and
  use the OS credential store. If a profile run shows a Keychain prompt, treat
  it as an automation failure and verify that env var is present in
  `--child-env`.
- If synthetic send smoke reaches `send=failed` while login, sync, and timeline
  are otherwise ready, check that the product room list excludes non-joined
  rooms before QA timeline sampling. Matrix SDK `Room::send` requires joined
  room state, and a left room with visible history can otherwise become the
  active QA room.

## Local Homeserver QA Failures

- Installing Conduit or Tuwunel from source with `cargo install --git` must set
  `RUMA_UNSTABLE_EXHAUSTIVE_TYPES=1`. Without it, Ruma marks many public API
  structs as non-exhaustive and both homeservers fail to compile with
  `E0639: cannot create non-exhaustive struct using struct expression`.
- On macOS, install Tuwunel with `--no-default-features` unless a Linux-oriented
  build profile is intentional. The default feature set includes deployment
  features such as `systemd`/`io_uring` that are not useful for local desktop QA.
