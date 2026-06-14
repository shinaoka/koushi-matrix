# Agent Notes

This is the operational entry file for agents and QA automation in this local
environment. It records setup commands, troubleshooting, and environment
footguns. Durable repository rules do not live here.

## Read Order

1. [REPOSITORY_RULES.md](REPOSITORY_RULES.md) - root durable rules for this
   repository.
2. [docs/architecture/overview.md](docs/architecture/overview.md) - long-term
   architecture, layer ownership, runtime, security, and QA model.
3. [docs/architecture/state-machine.md](docs/architecture/state-machine.md) -
   normative reducer state-machine diagrams and guard notes.
4. [docs/architecture/i18n.md](docs/architecture/i18n.md) - Rust-owned
   locale/display profile, catalog, pseudo-locale, RTL, and i18n gates.
5. [docs/policies/engineering-rules.md](docs/policies/engineering-rules.md) -
   detailed policy extension for secrets, logging, QA automation, and gates.
6. The relevant dated implementation plan under `docs/superpowers/plans/`.

When an operational note here hardens into a durable rule, promote it to
`REPOSITORY_RULES.md` or `docs/policies/engineering-rules.md` and keep only the
local how-to detail here.

## Current Implementation Plans

All agents implementing the headless core runtime follow
[docs/superpowers/plans/2026-06-12-headless-core-runtime-implementation.md](docs/superpowers/plans/2026-06-12-headless-core-runtime-implementation.md).
All agents implementing the Phase 10+ product surface and release roadmap
follow
[docs/superpowers/plans/2026-06-13-roadmap-phases-10-18.md](docs/superpowers/plans/2026-06-13-roadmap-phases-10-18.md).
All agents implementing local GUI room/space/reply operations follow
[docs/superpowers/plans/2026-06-13-local-gui-basic-operations.md](docs/superpowers/plans/2026-06-13-local-gui-basic-operations.md).
All agents implementing Rust-owned settings Phase A follow
[docs/superpowers/plans/2026-06-14-rust-owned-settings-phase-a.md](docs/superpowers/plans/2026-06-14-rust-owned-settings-phase-a.md).
All agents implementing the headless i18n substrate follow
[docs/superpowers/plans/2026-06-14-i18n-substrate-phase-a.md](docs/superpowers/plans/2026-06-14-i18n-substrate-phase-a.md).

## Rust-Owned Settings Notes

- Settings product state lives in `matrix-desktop-state::AppState.settings`.
  GUI work may render it and dispatch `update_settings`, but must not make
  locale, theme, font/emoji, or composer-send shortcut preferences a React or
  localStorage source of truth.
- Locale/display behavior is resolved by
  `matrix_desktop_state::resolve_locale_display_profile`. GUI components may
  consume the resulting `lang`, `dir`, catalog locale, pseudo-locale mode,
  platform, and modifier labels, but must not parse raw language tags or own
  fallback locale rules.
- Composer key behavior belongs to the Rust-owned resolver in
  `matrix-desktop-state`, shared by main, thread, and edit composer surfaces.
  GUI code normalizes DOM/native key input into typed resolver facts and then
  dispatches/renders the returned action.
- When `AppState.settings` or any settings enum changes, update the Tauri DTO,
  TypeScript domain types, `browserFakeApi` defaults, `tauriIpcMock`, app
  harness snapshots, and the DTO serialization-contract test in the same
  change. Headless mock snapshots do not automatically inherit Rust fields.
- The settings file is a non-secret JSON store under the core data directory
  (`settings/settings.json`). Do not route it through the credential store and
  do not add Matrix IDs, message content, raw SDK errors, credentials, tokens,
  recovery material, SDK store keys, or search-index keys to it.

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

## Headless UI (Playwright) Flakes

- `e2e/basic-operations.spec.ts:81` ("submitting the composer in reply mode
  invokes send_reply, not send_text") is flaky in the FULL `test:ui-headless`
  run but passes reliably when that spec file is run in isolation
  (`npx playwright test e2e/basic-operations.spec.ts`). Root cause is a
  test-layer timing race, not a product bug: the App's snapshot refresh
  (`get_snapshot`) returns the harness's static Plain `readySnapshot`, which can
  land after the reply-target click and momentarily reset the composer mode to
  Plain so the submit dispatches `send_text`. It reproduces on a clean checkout
  (predates the 2026-06-14 rules-compliance remediation) and is amplified by
  parallel-file worker contention on the shared Vite harness server. Workaround
  while it is unfixed: run the reply specs in isolation, or `--workers=1`.
  A durable fix should make the harness `get_snapshot` response consistent with
  the reply lifecycle (or have the App refresh from owned state, not a static
  mock). The `reply send does not repair product state by cancelling reply mode`
  regression added in that remediation passes deterministically in isolation.

## Linux GUI QA Container

- Build the committed lane image with
  `docker build -f docker/linux-gui.Dockerfile -t matrix-desktop-linux-gui:basic-ops .`
- The committed image includes `conduit`, `tuwunel`, and `zstd` so the
  `--scenario=local-login` and `--scenario=local-send` lanes can run against
  local homeservers entirely inside the container.
- The Docker recipe pins Rust toolchain `1.96.0` for reproducibility.
- The lane image includes `libnss-wrapper` so the numeric container UID can be
  given a temporary passwd/group entry during DBus-authenticated GUI smoke.
- Run the lane from the repo root with the workspace mounted at `/work`:
  `docker run --rm -it --shm-size=2g -u "$(id -u):$(id -g)" -v "$PWD:/work" -v /tmp/matrix-desktop-cargo-home:/tmp/cargo-home -v /tmp/matrix-desktop-gui-target:/tmp/matrix-desktop-gui-target -v /tmp/matrix-desktop-npm-cache:/tmp/npm-cache -w /work -e HOME=/tmp -e RUSTUP_HOME=/opt/rustup -e CARGO_HOME=/tmp/cargo-home -e CARGO_TARGET_DIR=/tmp/matrix-desktop-gui-target -e NPM_CONFIG_CACHE=/tmp/npm-cache -e PATH=/opt/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin matrix-desktop-linux-gui:basic-ops bash -c 'export RUSTC="$(rustup which rustc)"; export RUSTDOC="$(rustup which rustdoc)"; npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-send --server=conduit --artifact-dir=/work/artifacts/linux-gui-local-send-docker --timeout-ms=180000'`
- The runner writes artifacts to `artifacts/linux-gui-local-send-docker/` inside the mounted
  repo. Keep that directory ignored and inspect the run log and screenshots
  there when a lane fails.
- Faster Ubuntu 24.04 host loop:
  one-time package install still needs `sudo`/root, but tests and smoke then
  run as a normal user. Install the host packages with
  `sudo apt-get update && sudo apt-get install -y --no-install-recommends build-essential ca-certificates curl dbus-x11 file fontconfig fonts-dejavu-core fonts-noto-color-emoji fonts-noto-core git libayatana-appindicator3-dev libnss-wrapper libssl-dev libwebkit2gtk-4.1-dev libxdo-dev librsvg2-dev pkg-config webkit2gtk-driver xvfb`, then install the driver with `cargo install tauri-driver --locked`. Fast checks are
  `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml`,
  `node scripts/desktop-linux-gui-qa.mjs --check-tools`, and
  `npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-login --server=conduit --artifact-dir=artifacts/linux-gui-local-login-host --timeout-ms=180000` or
  `npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-send --server=conduit --artifact-dir=artifacts/linux-gui-local-send-host --timeout-ms=180000`.
  Docker remains the reproducible release/CI gate.

## Fast Linux GUI Inner Loop

- After the one-time host package install, run the GUI QA lanes as a normal
  user; no `su` or root shell is needed for the fast loop.
- Prepend the local homeserver binaries when iterating so the host lanes use
  the checked-in QA binaries first:
  `export PATH=/tmp/matrix-desktop-local-qa-bin:$PATH`
- Build the debug app once, then reuse it with `--skip-build` (optionally
  `--app-binary=PATH`) so each scenario trial skips the full Tauri rebuild:
  `npm --prefix apps/desktop run tauri build -- --debug --no-bundle`, then
  `PATH=/tmp/matrix-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-create-room --server=conduit --skip-build --artifact-dir=artifacts/linux-gui-local-create-room-fast --timeout-ms=180000`
- Settings/composer-shortcut GUI changes use the same fast loop with
  `--scenario=local-settings`. This opens the real Settings UI, changes the
  Rust-owned composer shortcut and theme settings, and waits for
  `aria-pressed="true"` / `data-theme="dark"` from the snapshot-driven UI:
  `PATH=/tmp/matrix-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-settings --server=conduit --skip-build --artifact-dir=artifacts/linux-gui-local-settings-fast --timeout-ms=180000`
- When you only need a quick window-state sanity check, use the lane's cheap
  QA title helpers such as `--qa-title-ready` and `--qa-title-send-ready`
  before starting a full scenario run.
- Use focused scenarios first. Keep the artifact directories scenario-specific
  so retries do not blur login and send results:

  ```bash
  PATH=/tmp/matrix-desktop-local-qa-bin:$PATH \
    node scripts/desktop-linux-gui-qa.mjs --check-tools

  PATH=/tmp/matrix-desktop-local-qa-bin:$PATH \
    node scripts/desktop-linux-gui-qa.mjs --list

  PATH=/tmp/matrix-desktop-local-qa-bin:$PATH \
    npm --prefix apps/desktop run qa:linux-gui -- \
      --scenario=local-login \
      --server=conduit \
      --artifact-dir=artifacts/linux-gui-local-login-host \
      --timeout-ms=180000

  PATH=/tmp/matrix-desktop-local-qa-bin:$PATH \
    npm --prefix apps/desktop run qa:linux-gui -- \
      --scenario=local-send \
      --server=conduit \
      --artifact-dir=artifacts/linux-gui-local-send-host \
      --timeout-ms=180000
  ```
- Reuse the existing Cargo, npm, and GUI target caches during the inner loop;
  do not rebuild the Docker image for every trial.
- Run Docker only when you need the committed reproducible lane or want to
  prove the release/CI recipe end to end. It is not the default fast
  iteration path.

## Linux GUI Local Operation Failures

- `--skip-build` reuses an existing debug binary, but the QA window-title
  tokens (`matrix-desktop qa session=...`) are baked into the frontend at build
  time behind `VITE_MATRIX_DESKTOP_QA_TITLE=1`. A binary built without that env
  shows the normal product title (e.g. `matrix-desktop · 1 unread`) instead, so
  the lane's `waitForLocalLoginReady` times out with "local GUI login did not
  reach a ready state. Last title: matrix-desktop · 1 unread". The runner's own
  build sets this env; when pre-building manually for `--skip-build`
  (`npm --prefix apps/desktop run tauri build -- --debug --no-bundle`), also set
  `VITE_MATRIX_DESKTOP_QA_TITLE=1`, or run one lane without `--skip-build` first
  to produce a QA-title binary the remaining `--skip-build` lanes can reuse.
- The Tauri snapshot is a hand-maintained DTO
  (`apps/desktop/src-tauri/src/dto.rs`, `FrontendAppState` / `From<AppState>`),
  NOT a passthrough of `AppState`. When `AppState` gains a field (e.g.
  `basic_operation`), the DTO must be extended in the same change, or the
  serialized snapshot silently omits it and the React UI crashes the moment it
  reads the missing field. Symptom: clicking a control blanks the WebView and
  `window.onerror` reports `undefined is not an object (evaluating
  'e.state.basic_operation.kind')`. Headless tests that use the browser fake or
  mock IPC will NOT catch this (they build their own snapshots); only the real
  Tauri lane or the `dto.rs` serialization-contract test does. Extend that test
  when adding `AppState` fields.
- WebDriver `waitForDisplayed`/`click` does NOT reveal hover-gated controls.
  Timeline row actions (`.message-action` inside `.message-actions`) are
  `opacity:0` until `.message:hover`/`:focus-within`, so a direct
  `waitForDisplayed` on the reply button times out ("still not displayed") even
  though the headless Playwright tier passes (its click implicitly hovers).
  Move the pointer first: `await el.waitForExist(); await el.moveTo(); await
  el.waitForDisplayed(); await el.click();`.
- A reply must target a MESSAGE event, not a state event. The timeline includes
  state events (room create, membership) that carry no body; the SDK's
  `make_reply_event` rejects them (app stderr `make_reply_event failed:
  StateEvent`, surfaced as `send=failed`). `TimelineItemRow` therefore gates the
  reply affordance on `item.body !== null`, so only message rows are replyable.
  A `local-reply` lane must send/target a message and reply to that row, not the
  first event row in a fresh room (whose first events are state events).

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
