# Troubleshooting

Symptoms that have cost real time here, and what actually fixed them. Grouped by
the lane that shows the symptom. Lane commands are in
[qa-lanes.md](qa-lanes.md); dated investigation records are in
[history.md](history.md).

## Browser-headless harness

- **Timeline rows vanish mid-assertion, with harness seed content in the failure
  snapshot.** The app harness boot loop re-emits its generation-1 seed
  `InitialItems` every 25ms (up to 40 attempts) until the seed row is visible in
  the DOM. Until 2026-07-30 that visibility check was its ONLY exit, so a spec
  that replaced the timeline while the loop was still running (e.g.
  `timeline-thread-latest-placement.spec.ts` pushing generation 101 right after
  `gotoReadyApp`) had its rows overwritten by a late seed re-emit: a row passes
  `toHaveCount(1)` and vanishes
  one assertion later. `appHarnessMain.tsx` now sets `externalCoreEventPushSeen`
  inside `pushCoreEvent` and the boot loop stops re-emitting once any spec push
  has happened. If this recurs, look for a new path that bypasses
  `pushCoreEvent` instead of adding waits to specs.
- **A diagnostics assertion passes locally and fails on CI.** Assert
  scroll/render diagnostics on a CUMULATIVE counter, never on `latestFrame`.
  `TimelineScrollDiagnostics.latestFrame` is overwritten every frame, and
  `TimelineView` emits frames carrying zeroes right after the frame you care
  about (the height-compensation effect and the active-scroll range
  recomposition both report `changedMeasuredRowCount: 0`). An assertion like
  `latestFrame.changedMeasuredRowCount > 0` therefore depends on the read landing
  before the next frame. Counters such as `heightModelCommits`,
  `measurementFlushes`, and `changedMeasuredRows` are monotonic and
  baseline-subtracted by the harness. `pendingMeasuredRows` is NOT one of them —
  it is a gauge that is decremented on flush and reset to zero. When a new
  per-frame fact needs a test, add the matching cumulative counter in
  `apps/desktop/src/domain/timelineScrollDiagnostics.ts` and subtract it in
  `harnessMain.tsx` rather than reaching into `latestFrame`.
- **Prepend anchor restoration never engages.** A bare `node.scrollTop = X` is a
  PROGRAMMATIC scroll: the component keeps `live-edge` viewport intent and snaps
  back to the bottom. To simulate a real user scroll-up, dispatch a `WheelEvent`
  (or container `pointerdown`) so `userScrollInputPendingRef` is set, then a
  `scroll` event. The component leaves live-edge only on user-driven input.
- **A dependent `expect.poll` times out after a scroll.** After changing
  `scrollTop`, always `dispatchEvent(new Event("scroll"))` explicitly. Do not
  rely on the browser's native async scroll event; it is not delivered reliably
  in cold or loaded headless Chromium.
- **Later counters are polluted after a large scroll jump.** Do not gate a
  diagnostics measurement window with a fixed `waitAnimationFrames(n)`: the
  jump's range-recomposition frames can spill past the wait under load. Wait on a
  CONDITION instead (e.g. poll until `scrollDiagnostics().scrollFrames` stops
  changing) before `resetScrollDiagnostics()`.
- **`locator.click()` broadly times out waiting for elements to be stable.**
  First prove whether Chromium headless is producing animation frames: run a
  blank-page `requestAnimationFrame` probe. In this local environment, headless
  Chromium can leave `requestAnimationFrame` suspended even when `setTimeout`
  works, which makes Playwright actionability fail although DOM boxes are stable.
  Do not patch product code or one fixture at a time for that symptom. For local
  diagnostic proof, use Xvfb with headed Chromium and report that mode
  explicitly:
  `xvfb-run -a npm --prefix apps/desktop exec -- playwright test --headed --config apps/desktop/playwright.config.ts --workers=1`.
- **A seeded timeline row never appears and a later unrelated assertion times
  out.** When a Playwright helper seeds event-driven timeline rows with fake
  `CoreEvent::Timeline::InitialItems`, make the helper wait until every target
  `data-item-id` is visible and fail on timeout. Do not fire a fixed number of
  events and let the test continue.
- **An i18n spec updates `lang`/`dir` but leaves the seed row visible.** For
  tests that first push a locale/profile snapshot and then mutate the
  event-driven timeline, prefer updating the already-seeded room row with
  `ItemsUpdated.Set` at generation `1`. A one-off `InitialItems` emitted around
  the same snapshot refresh can be swallowed by harness timing.
- **A red full local Playwright run is not automatically scope for the active
  PR.** First classify each failure: introduced by the current diff, changed-area
  regression, shared fixture/harness drift, local browser/actionability
  environment, or unrelated product backlog. Fix it in the current PR only when
  it is introduced by the diff, blocks a required gate, or shares the same
  contract boundary the PR is already repairing. Otherwise record the inventory
  and move it to a dedicated UI-harness stabilization branch/issue.

## Linux GUI (WebDriver) lane

- **A hover-gated control "still not displayed".** WebDriver
  `waitForDisplayed`/`click` does NOT reveal hover-gated controls. Timeline row
  actions (`.message-action` inside `.message-actions`) are `opacity:0` until
  `.message:hover`/`:focus-within`, so a direct `waitForDisplayed` on the reply
  button times out even though the headless Playwright tier passes (its click
  implicitly hovers). Move the pointer first: `await el.waitForExist(); await
  el.moveTo(); await el.waitForDisplayed(); await el.click();`.
- **`element not interactable` on a visible menu item.** WebDriver native clicks
  can be flaky on nested absolute menu items inside hover-gated timeline actions.
  If the menu is visible and exact labels are present, use a scenario-local
  helper that finds the visible `button[role="menuitem"]` by exact text and
  dispatches a DOM click. Keep this fallback limited to GUI QA plumbing; product
  code must still use typed Rust commands.
- **Message-action QA cannot find its redaction seed.** WebKit's bulk
  `setValue()` on the contenteditable can change case or drop characters before
  sending. The redaction seed uses balanced native key actions with explicit
  Shift lifetimes and verifies the exact editable value before Enter; do not
  loosen the expected message text. The lane also checks the Rust default
  `hide_redacted=true`, reveals the placeholder, then hides it again rather
  than assuming deleted messages are initially visible.
- **A `datetime-local` control stays empty.** WebDriverIO/WebKit `setValue()` did
  not populate it in the date-jump lane: the DOM input stayed `valueLength=0`,
  `valid=false`, and the app title stayed `panel=closed focused=closed`. Use the
  lane's `setDatetimeLocalValue` helper, which sets the native value property and
  dispatches `input`/`change`, then verify with `timelineDateJumpDiagnostics`
  before clicking submit. Reuse the same helper for scheduled-send controls. The
  QA title includes `focused=closed|opening|open` so future failures distinguish
  command dispatch and focused-context state from plain DOM text waits.
- **The lane times out at login with a normal product title.** See
  [environment.md](environment.md#reusing-a-debug-build) — the QA title tokens
  need `VITE_KOUSHI_QA_TITLE=1` at build time.
- **The WebView blanks after a click.** A missing Tauri DTO field; see
  [state-ownership.md](state-ownership.md#snapshot-and-wire-contract-mirrors).
- If a lane fails, inspect the scenario-specific artifact run log and screenshots
  under its `--artifact-dir`.

## macOS GUI smoke

- `AppleScript timed out while controlling System Events` — grant Accessibility
  permission to the app running the agent (Claude Code, Terminal, iTerm), then
  restart that app.
- If Accessibility is already enabled and the timeout repeats, check Privacy &
  Security > Automation and allow the same app to control `System Events`.
  Restart the agent app after changing either permission.
- A repeated timeout can also be caused by AppleScript code, not permissions. In
  this repo, `process <variable>` hung when resolving the Tauri process. Use
  `first process whose name is <variable>` for variable process names.
- If screenshot capture is blocked, also grant Screen Recording permission.
- In Tauri dev mode the macOS process name can be `matrix-desktop-app`, while the
  product/window title is `Koushi`. GUI automation must check both names.
- Failed GUI smoke runs must clean up the full process group. A stale Vite
  process leaves port `5173` occupied and makes the next `tauri dev` fail. After
  a manual Ctrl-C, verify `lsof -nP -iTCP:5173 -sTCP:LISTEN` is empty before
  retrying.
- Do not use `Cmd+Q` to stop the Tauri app from GUI smoke. If focus slips, the
  shortcut reaches the app running the agent and raises its own quit confirmation
  dialog, which blocks unattended automation. Let the script's process-group
  cleanup stop `tauri dev` and the app instead.
- In this environment, starting `qa:mac-gui -- --real-login-from-stdin` through a
  non-interactive `exec_command` can deliver immediate stdin EOF. Use a PTY with
  terminal echo disabled, such as `stty -echo; npm --prefix apps/desktop run
  qa:mac-gui -- --real-login-from-stdin; exit_code=$?; stty echo; exit
  $exit_code`, then send the credential lines through stdin.

## Real-account smoke

- **`password-login-smoke --real-account-qa` fails at sync but
  `--check-room-list` succeeds.** Isolate the restore path first. A no-store `restore_session` can diverge from the product path;
  real-account QA should restore with a temporary encrypted SQLite SDK store,
  cache path, and encrypted search index path.
- **`qa.log` missing after a fast successful exit.** Treat it as a regression in
  the runner; it writes the log synchronously before leak checks and exit
  handling.
- **`deadpool-runtime` panics with `there is no reactor running`.** Store-backed
  Matrix SDK sessions must be dropped while a Tokio runtime context is entered.
- **`send=failed` while login, sync, and timeline are otherwise ready.** Check
  that the product room list excludes non-joined rooms before QA timeline
  sampling. Matrix SDK `Room::send` requires joined room state, and a left room
  with visible history can otherwise become the active QA room.
- **Sparse accounts have no visible timeline items** in the automatically
  selected room. Use `--allow-empty-timeline` for those; keep the strict
  `timeline_items > 0` signal for normal smoke.

## Local homeserver core QA

- **`avatar_demand` stays at one reader after a successful large fixture seed.**
  Tuwunel 1.7.1 returned one receipt user in the initial Simplified Sliding Sync
  response in this test, while Synapse returned the full population. Its
  [`pack_receipts`](https://github.com/matrix-construct/tuwunel/blob/v1.7.1/src/service/rooms/read_receipt/mod.rs#L276-L304)
  inserts each receipt map by event ID, replacing previous users for that event
  instead of merging the nested receipt-type/user maps. Seed HTTP success is not
  receipt-readback proof. Keep the failing population gate; do not
  manufacture readers in Core or add a sync fallback. The active investigation
  and exact evidence are in the 2026-09-09 remaining-issues batch worklog.

- **Synapse returns 429 while populating one room despite high local/remote join limits.**
  `rc_joins_per_room` is separate from `rc_joins.local` and `.remote`; the pinned
  Synapse defaults it to one join/second with a burst of ten. The disposable QA
  config raises this separate limit too. Use a newly generated fixture config
  when testing that change; do not alter production or unrelated servers.

- **`login A: timed out waiting for LoggedIn event`.** Read the `phase=…` token
  in the message before re-running; it names the session phase and has
  identified the cause in a single run. `phase=rechecking_trust` means the
  session left the bootstrap gate, entered `Provisional { RecheckingTrust }`, and
  never promoted, so `LoggedIn` stayed held in the actor's pending-ready events.
  Headless login timeouts also include an allowlisted `trust_path` of stage
  tokens only; read it before rerunning. The full investigation is in
  [history.md](history.md#login-timeout-investigation-334-375).
- Trust-recheck coalescing is lossless by contract: keep at most one query in
  flight, remember one pending demand, replay it after query settlement, and if a
  projection ack does not match the reducer's current state, discard that obsolete
  transition and run the pending query. A matching Ready/Locked ack may satisfy
  the redundant demand. Clear pending demand on provisional-session teardown. An
  ack for the exact generation/transition that does not reach Ready/Locked always
  makes that transition obsolete: clear it whether demand arrived before or after
  the ack, then start any already-pending query. Focused gates:

```bash
cargo test -p koushi-core --lib explicit_trust_recheck
cargo test -p koushi-core --lib projection_mismatch_before_explicit_recheck_does_not_block_later_query
cargo test -p koushi-core --lib runtime::tests::authoritative_trust_runs_through_app_actor_ack_and_restarts_real_children
```

  Do not ship a promotion change with that last test red: re-emitting the SDK
  subscriber's current value can feed `Unknown` into promotion at the wrong
  moment.

- A concurrency-sensitive race in this area reproduces under CPU restriction.
  Restricting the `media` scenario to two CPUs (`taskset -c 0,1`) reliably
  surfaced the recheck stall that ran green on a full machine.
- `complete_new_identity_gate_for_qa` settles its `ConfirmSessionBootstrapSaved`
  confirmation and surfaces a correlated failure as its own error. It previously
  returned without observing the outcome, which made a failed confirmation
  indistinguishable from a stall — after it had already printed
  `gate_new_identity_bootstrap=ok`.
