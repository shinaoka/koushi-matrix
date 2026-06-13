# Roadmap: Phases 10–18 (to first release)

Date: 2026-06-13
Status: active plan. Supersedes
`2026-06-13-phase-10-ui-headless-product-surface.md` (its Phases 10–14 are
absorbed, renumbered, and extended here; that document now redirects here).

Binding context (unchanged): canon =
[overview.md](../../architecture/overview.md) +
[engineering-rules.md](../../policies/engineering-rules.md); Redesign
Protocol and Model Assignment from
[2026-06-12-headless-core-runtime-implementation.md](2026-06-12-headless-core-runtime-implementation.md)
apply to every phase below — implementers stop on canon gaps, the strongest
model amends canon and reviews phase exits, reviewers re-execute gates
themselves.

## Environment strategy

- Phases 10–13 run on the current macOS environment: everything is headless
  (Vitest, Playwright + mock IPC, local homeserver QA, real homeserver QA).
- Phase 14 builds the Linux virtual-display lane. From Phase 14 on, GUI
  integration work runs primarily on Linux (Xvfb + `tauri-driver`); macOS
  remains for attended WKWebView/menu/Keychain smoke and for release
  signing/notarization in Phase 18.
- The switch to Linux as the primary agent environment happens at the
  Phase 14 exit if the lane is green; the canon already accepts this.

## Standing gates (every phase exit, executed by the reviewer)

```bash
cargo test -p matrix-desktop-core --lib
cargo test -p matrix-desktop-sdk -p matrix-desktop-state -p matrix-desktop-search -p matrix-desktop-key
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
cargo check --target wasm32-unknown-unknown -p matrix-desktop-state -p matrix-desktop-search
npm --prefix apps/desktop run typecheck && npm --prefix apps/desktop run test
npm --prefix apps/desktop run test:ui-headless
npm --prefix apps/desktop run test:ipc-contract
npm --prefix apps/desktop run qa:secret-scan
node scripts/desktop-release-gate-check.mjs --no-compile
```

Plus when Matrix behavior changed: `qa:headless-local -- --server=both`
(all four legs). Plus before any release/GUI-confidence claim:
`qa:real-homeserver` (login budget discipline; expected tokens per the
Phase 8/9 changelog). Docs-sync check closes every phase.

## Open product decisions (need the user; block the marked phases only)

| # | Decision | Default if unanswered | Blocks |
|---|---|---|---|
| D1 | Reactions (emoji) in first release? | No (post-release) | P12 composer/timeline affordances |
| D2 | Read receipts / read markers UI? | Unread counts only, no per-message receipts | P12 timeline |
| D3 | Media: view/download attachments? Send? | View/download filenames only via search results; no media send | P12, P17 |
| D4 | Desktop notifications on new messages? | Yes, OS notifications with redacted content option | P15 |
| D5 | Auto-update channel for releases? | No auto-update in first release; manual download | P18 |
| D6 | Identity server (vector.im) — 3PID lookup / email invites? | **DECIDED (user, 2026-06-13): no identity server in the first release.** Invites by MXID only, no 3PID binding. | P12 invite UI copy, P18 scope freeze |
| D7 | Emoji rendering | **DECIDED (user, 2026-06-13): emoji support is mandatory.** Bundle twemoji-colr (Element-style) for identical rendering on macOS/Windows/Linux; composer gets an emoji picker (P12); THIRD_PARTY_NOTICES carries the Mozilla Apache-2.0 (font) and Twitter CC-BY-4.0 (art) attributions (P18). Linux container still installs Noto Color Emoji as system fallback for non-bundled surfaces. | P12 picker, P14 fonts, P16 SAS display, P18 notices |

## Phase 10 — Headless UI contract and harness hardening

Goal: the browser harness can mount the real app shell so Phases 11–13 UI
work is verifiable without any native process.

- [ ] Extend `TauriIpcMock` with reusable snapshot/event/command-response
  helpers; strict sanitization of password/recovery/token-like fields in
  recorded invocations.
- [ ] Playwright guard: harness fails the run if any non-mock IPC or native
  Tauri path is touched.
- [ ] Harness mounts the full app shell (not just `TimelineView`) with
  synthetic snapshots; add `e2e/app-shell-headless.spec.ts`.
- [ ] Headless tests for command shapes: room selection, panel open/close,
  search entry, settings entry, logout. Names + non-secret arg shapes only.
- [ ] `coreEvents.generated.json` regeneration discipline: regenerate via
  the Rust contract test only; never hand-edit.
- [ ] CI-less enforcement: wire the standing gates into a single
  `npm run gates:all` convenience script (local + future CI entry point).

Exit gate: real app shell renders and reacts in headless Chrome via mock
IPC only. Gap watchlist: app shell may have hidden Tauri-API references
that break in a plain browser (window decorations, menu APIs) — isolate
behind the transport module.

## Phase 11 — Thread model completion (headless-first, core before UI)

Goal: Element-default thread behavior — collapsed in the room timeline,
expanded on demand — fully proven headless. Canon amendment first
(Redesign Protocol): room live timelines are built with
`hide_threaded_events: true`; thread roots carry a summary; expansion is a
`TimelineKind::Thread` subscription.

Core (strongest-model canon edit, then implementer):
- [ ] Amend overview.md TimelineActor bullet + spec `TimelineItem`:
  `thread_root: Option<String>` (event id this item replies into, for
  in-thread items) and `thread_summary: Option<ThreadSummaryDto>`
  (`reply_count`, `latest_sender`, `latest_body_preview`, `latest_timestamp_ms`)
  on root items. Bump Last amended; changelog entry.
- [ ] `timeline.rs`: Room live timelines switch to
  `hide_threaded_events: true`; project SDK `ThreadSummary` into the DTO;
  summary updates arrive as `Set` diffs on the root item.
- [ ] Thread timelines (`TimelineKind::Thread`): verify subscribe / diffs /
  backward paginate / send-into-thread end to end (send queue threading —
  check the SDK reply-within-thread API; escalate if the send-queue thread
  relation needs design).
- [ ] Local QA, all four legs: A sends root, B replies in thread ×2;
  assert (a) replies do NOT appear in either room live timeline,
  (b) root item gains `thread_summary.reply_count=2` via Set diff,
  (c) thread subscription delivers root + both replies in order,
  (d) B sends in-thread reply that A's subscribed thread timeline receives
  live, (e) thread backward pagination to EndReached. Print tokens
  (`thread_hidden=ok thread_summary=ok thread_recv=ok thread_paginate=end_reached`).
- [ ] Real homeserver QA: extend with a thread leg in the QA room (root +
  reply + summary + thread restore after store-restore).
- [ ] Unit tests: summary projection, hide flag, thread-send mapping,
  Debug redaction of `latest_body_preview` carriers (preview is visible UI
  state in events; must still never reach logs/Debug).

UI (headless):
- [ ] Timeline renders the thread chip ("N replies, latest …") on roots;
  click issues `Subscribe` (thread key) — assert command shape in harness.
- [ ] Thread panel rendering from a thread-key timeline store instance
  (right panel placement lands fully in Phase 12; here a minimal panel
  proves the data path).
- [ ] Playwright: chip click → thread items render; room timeline shows no
  thread replies; composer-in-thread sends with the thread key.

Exit gate: standing gates + four local legs with thread tokens + real-run
thread tokens. Gap watchlist: SDK ThreadSummary completeness on
LegacySync (summaries may be sliding-sync-fed; verify on the forced-legacy
leg and escalate if legacy needs its own aggregation); Conduit/Tuwunel
thread support differences.

## Phase 12 — Product surface (Element-like three-pane desktop UX)

Goal: the visible product: left navigation (spaces rail + room list +
global DMs), center timeline + composer, contextual right panel — all
render-only over existing commands/events, verified headless.

- [ ] `DesktopShell.tsx`: three-pane layout, narrow-width drawer behavior,
  focus management (focus returns on panel close).
- [ ] Left nav: space rail (space-filtered lists per reducer semantics),
  room list with unread badges and DM section (compose_sidebar exists in
  matrix-desktop-state — reuse), account switcher entry
  (`QuerySavedSessions`/`SwitchAccount`).
- [ ] Center: timeline (Phase 7/11 component), composer with drafts
  (reducer `ComposerDraftChanged`), edit/redact affordances on own
  messages (context menu), send states from local echo → SendCompleted.
- [ ] Emoji (D7, mandatory): bundle twemoji-colr webfont, render message
  bodies and UI emoji with it; composer emoji picker (searchable, recent
  section, keyboard accessible); headless tests for picker insertion and
  consistent glyph rendering (screenshot-free DOM/font assertions).
- [ ] Right panel: thread panel (Phase 11 data path), room info stub,
  search results panel wired to `SearchCommand::Query` + result navigation
  (select room + focused timeline jump via `TimelineKind::Focused` —
  forward pagination already in core).
- [ ] Settings panel: account info, logout, recovery status surface
  (NeedsRecovery → recovery key entry → SubmitRecovery flow, secrets
  one-way), saved sessions list.
- [ ] Login screen: discovery → password → NeedsRecovery interstitial;
  no secret ever rendered back; never logged.
- [ ] Keyboard shortcuts registry + Element parity table doc
  (docs/qa/shortcut-parity.md) — implement the cheap ones (room switch,
  search focus, thread close), record deviations.
- [ ] All fixture data synthetic; Vitest component tests + Playwright DOM
  tests for: panel switching, drawer behavior, focus return, search
  result → focused-timeline jump, recovery flow command shapes, account
  switcher flows.
- [ ] D1/D2/D3 decisions applied (defaults if unanswered).

Exit gate: standing gates; a scripted headless walkthrough spec
(login → rooms → send → thread → search → settings → logout) passes in
one Playwright run. Gap watchlist: focused-timeline UX needs core
`Focused` subscription exercised in local QA (add a leg token if Phase 11
didn't); AppState snapshot churn under large room lists (coalescing
validation with 500-room synthetic snapshots).

## Phase 13 — Transport integration hardening

Goal: adapter, TS contract, and runtime agree under stress, still headless.

- [ ] Rust tests for every Tauri command → CoreCommand route (table-driven;
  redacted failures asserted).
- [ ] Event-flood test: synthetic burst (1k diffs) through the adapter →
  webview event channel; assert coalescing, lag marker + snapshot resync
  path end to end (harness consumes the real serialized stream).
- [ ] Reconnect/offline surface: sync Reconnecting/Failed states render;
  Restart command path from the UI; send-queue offline behavior surfaced
  (queued sends visible as unsent local echoes; assert via mock + local QA
  leg with a stopped server if cheap, else unit-level).
- [ ] Multi-window decision recorded (single-window assumption documented
  in canon if we keep it).
- [ ] `qa:headless-local -- --server=both` + `qa:real-homeserver` rerun
  (this phase touches Matrix-facing paths).

Exit gate: standing gates + both QA tiers. Gap watchlist: Tauri event
throughput on large bursts (if the IPC channel saturates, the canon
backpressure rules need an adapter-side coalescer — escalate with
measurements).

## Phase 14 — Linux virtual-display lane (environment switch point)

Goal: agents drive the REAL Tauri app unattended under Xvfb on Linux;
primary GUI development moves to Linux at exit.

- [ ] Provision Linux env (container or VM): Rust toolchain, Node, tauri
  deps (`webkit2gtk`, `libayatana-appindicator`…), Xvfb, `tauri-driver`,
  WebdriverIO; Conduit/Tuwunel built with the documented flags (AGENTS.md
  caveats apply on Linux too — record deltas). Install Noto Color Emoji
  (D7): without it WebKitGTK renders tofu for emoji and Phase 16's emoji
  SAS display cannot be verified.
- [ ] Repo runs whole standing-gate suite on Linux (fix platform breaks:
  keychain → file credential store is debug-gated already; `keyring`
  crate's linux backend behavior documented; paths).
- [ ] `qa:linux-gui` script: Xvfb + tauri-driver + WebdriverIO drives the
  real app: window lifecycle, real IPC bridge round trip (command →
  CoreEvent → DOM), menu wiring, login-via-FIFO smoke against a local
  homeserver, scrollback in the real webview (WebKitGTK), clean process
  teardown. Screenshots only from synthetic accounts, stored in ignored
  artifacts.
- [ ] Keychain-prompt equivalent on Linux (secret service): the QA guard
  pattern ports (file credential store enforced; no DBus secret prompts in
  unattended runs).
- [ ] AGENTS.md: Linux lane setup + footguns section; canon amendment
  recording the lane as the primary GUI verification path.
- [ ] macOS attended smoke checklist doc updated to ONLY macOS-specific
  items (WKWebView rendering spot-check, OS menu accelerators, Keychain
  prompt suppression, notarized-build launch).

Exit gate: full gate suite + `qa:linux-gui` green on Linux, unattended.
Gap watchlist: WebKitGTK vs WKWebView rendering differences (track a
known-deltas doc); tauri-driver maturity (pin versions); Xvfb DPI/scroll
metrics affecting anchor assertions.

## Phase 15 — Desktop interaction completeness

Goal: the "desktop app" feel, still agent-verifiable (Linux lane + headless).

- [ ] OS notifications (D4): core emits a notification-worthy event surface
  (new message in non-focused room, respecting DM/mention precedence —
  canon amendment for the event shape first); adapter maps to OS
  notifications with a redacted-content option; headless tests assert the
  decision logic, Linux lane asserts a notification fires (DBus assert).
- [ ] Unread/badge wiring: dock/taskbar badge counts from AppState; window
  title unread hint (QA window title rules respected).
- [ ] Window state persistence (position/size/maximized — code exists in
  src-tauri; port to the new adapter and test on the Linux lane).
- [ ] Accessibility pass (minimal): focus order, ARIA roles on the three
  panes, keyboard-only walkthrough spec in Playwright.
- [ ] Shortcut parity table completed; deviations documented.

Exit gate: standing gates + Linux lane suite incl. notification/badge
assertions. Gap watchlist: notification content redaction policy (exact
fields allowed — needs a canon line mirroring the UI-state allowances).

## Phase 16 — Device verification and cross-signing (E2EE trust)

Goal: close the canon's declared open area. PRECONDITION: a dated spec
authored by the strongest model amending overview.md (AccountActor
children, commands/events, state surfaces) — implementation starts only
after that spec lands.

Spec must cover (minimum):
- [ ] Verification surfaces: own-device verification after login/restore
  (emoji SAS to another own device; recovery-key path already exists),
  user verification requests in DMs, device list with trust states.
- [ ] Commands/events sketch: `VerificationCommand::{Request, Accept,
  ConfirmSas, Cancel}`, `VerificationEvent::{Requested, SasEmojis{..},
  Completed, Cancelled, DeviceListUpdated}` — emoji payloads are
  identifiers, never key material; all redaction rules apply.
- [ ] Trust indicators in timeline/room state (shield states from the SDK)
  and their AppState projection.
Implementation after spec:
- [ ] AccountActor verification child actor over SDK verification API;
  unit tests with fake ports; redaction tests.
- [ ] Two-runtime local QA: A and B verify via emoji SAS (both runtimes
  exchange and confirm programmatically), assert trust state change and
  shield transitions on both ends, on both servers (check Conduit/Tuwunel
  cross-signing support; if a local server lacks it, that combination is
  documented and the real-homeserver leg becomes the gate).
- [ ] Real homeserver QA: cross-sign the QA device via recovery key
  (`recovery=completed` already proves secret storage access); verify a
  second ephemeral device end to end, then sign out both.
- [ ] UI: verification dialogs (emoji grid — rendering per D7; the 64 SAS
  emoji must be visually distinct on every supported platform), device
  list in settings, shield badges — headless + Linux lane specs.

Exit gate: standing gates + both QA tiers + Linux lane verification flow.
Gap watchlist: SDK verification API stream shapes; Conduit/Tuwunel
cross-signing gaps; UTD (unable-to-decrypt) handling decisions may emerge
— escalate, don't improvise.

## Phase 17 — Performance, reliability, soak

Goal: holds up on real accounts and long sessions.

- [ ] Virtualized room list and timeline rendering (windowing) with the
  anchor contract preserved; Playwright scroll tests against 5k-item
  synthetic timelines.
- [ ] Startup time budget: cold start to interactive < 3s on dev hardware
  with a 200-room synthetic store; measure store-restore path.
- [ ] Event-loop audits: no unbounded queues (re-verify capacities under
  burst), memory ceiling test over a 2h synthetic soak (scripted sends on
  local server, both backends).
- [ ] Real-account soak (attended kickoff, then unattended): 1h on
  matrix.org QA account with periodic sends/search; assert no UTDs on own
  messages, no queue overflows, memory stable; logout cleanup.
- [ ] Conduit transient room-list flake: root-cause or bound it (retry
  budget in QA runner, documented).
- [ ] Release-profile build performance check (`cargo build --release`
  timing recorded; binary size budget noted).

Exit gate: standing gates + soak reports in docs/qa (private-data-free).
Gap watchlist: SDK store growth (sqlite vacuum policy); search index size
on large accounts (D3 interacts here).

## Phase 18 — Distribution and release

Goal: signed, installable first release with release-grade gates.

- [ ] Versioning + changelog policy; `0.1.0` scope freeze against Product
  Scope (canon) + decisions D1–D5.
- [ ] macOS: signing + notarization pipeline (attended on macOS; secrets
  via local keychain profiles, never in repo/env wholesale), DMG/app
  bundle verification, Gatekeeper first-launch check (attended).
- [ ] Windows: build + `keyring` live credential-store ignored tests on a
  Windows runner; installer (NSIS/MSI per tauri-bundler), SmartScreen
  notes. (If no Windows hardware/runner is available, mark the Windows
  claim BLOCKED rather than shipping unverified — canon honesty rules.)
- [ ] Linux: AppImage/deb as a by-product of the Phase 14 lane (unsigned,
  documented).
- [ ] Release preflight: full gate suite + `qa:real-homeserver` +
  `release:preflight:strict` + secret scan + `git diff --check` + vendored
  SDK patch final review (drop obsolete patches; upstream PR drafts per
  docs/upstream roadmap).
- [ ] THIRD_PARTY_NOTICES regeneration; license audit for ported Element X
  code (engineering rule Build 1); twemoji-colr attributions (D7): the
  font © Mozilla Foundation under Apache 2.0, the Twemoji art © Twitter,
  Inc and contributors under CC-BY 4.0.
- [ ] Release smoke on a CLEAN machine/VM per OS: install → login (test
  account) → recovery → send/receive → logout (attended where OS dialogs
  are involved).
- [ ] Tag, build artifacts, publish per D5; post-release: mark this plan
  complete; open the post-release upstream/feedback plan
  (2026-06-12-upstream-feedback-roadmap.md takes over).

Exit gate: signed-build evidence on macOS, Windows evidence or explicit
BLOCKED, Linux artifact, all preflight gates green, no unreviewed vendor
deltas, canon Last-amended current, docs-sync clean repo-wide.

## Phase ordering rationale and dependencies

```
P10 harness ─► P11 threads(core→UI) ─► P12 surface ─► P13 transport ─► P14 Linux lane
                                                                        │ (env switch)
                                          P15 desktop ◄────────────────┘
                                          P16 verification (spec gate)
                                          P17 perf/soak
                                          P18 release
```
P15–P17 may interleave after P14 (independent workstreams) but P16's spec
must precede its implementation, and P18 starts only when P15–P17 exits
are green.

## Changelog

- 2026-06-13: roadmap created (absorbs and supersedes the Phase 10 UI
  plan; extends to release as Phases 10–18; thread model phase added from
  the 2026-06-13 timeline-thread review; Linux lane becomes the
  environment switch point at Phase 14).
