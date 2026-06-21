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
All agents planning or implementing the remaining umbrella #12 work follow the
Core Batch A / GUI Batch B split in
[docs/superpowers/specs/2026-06-15-remaining-core-phase-a-batch-design.md](docs/superpowers/specs/2026-06-15-remaining-core-phase-a-batch-design.md):
batch Rust-owned Phase A contracts first, then serialize the shared GUI
surface, then run the #9/#31 integration gate.
The umbrella #12 implementation batch follows
[docs/superpowers/plans/2026-06-15-remaining-core-phase-a-batch-implementation.md](docs/superpowers/plans/2026-06-15-remaining-core-phase-a-batch-implementation.md)
after that plan is approved. Before starting each new task in that batch,
refresh open GitHub issues and apply the plan's issue reconciliation addendum;
new GUI-only presentation items such as space tooltips do not bypass the
Rust-owned Phase A rule for product behavior.
All agents implementing media/file timeline support follow
[docs/superpowers/plans/2026-06-15-media-phase-a.md](docs/superpowers/plans/2026-06-15-media-phase-a.md)
for Phase A Rust/headless work before Phase B GUI wiring.
All agents implementing read receipts, read markers, typing, and presence
follow
[docs/superpowers/plans/2026-06-15-live-signals-phase-a.md](docs/superpowers/plans/2026-06-15-live-signals-phase-a.md)
for Phase A Rust/headless work before Phase B GUI wiring.
Phase B GUI/browser-headless work for the same issue follows
[docs/superpowers/plans/2026-06-15-live-signals-phase-b-gui.md](docs/superpowers/plans/2026-06-15-live-signals-phase-b-gui.md).
All agents implementing E2EE trust Phase A state-machine contracts follow
[docs/superpowers/plans/2026-06-14-e2ee-trust-phase-a.md](docs/superpowers/plans/2026-06-14-e2ee-trust-phase-a.md).
All agents implementing Rust-owned settings Phase A follow
[docs/superpowers/plans/2026-06-14-rust-owned-settings-phase-a.md](docs/superpowers/plans/2026-06-14-rust-owned-settings-phase-a.md).
All agents implementing the headless i18n substrate follow
[docs/superpowers/plans/2026-06-14-i18n-substrate-phase-a.md](docs/superpowers/plans/2026-06-14-i18n-substrate-phase-a.md).
All agents implementing the i18n GUI wiring follow
[docs/superpowers/plans/2026-06-14-i18n-substrate-phase-b.md](docs/superpowers/plans/2026-06-14-i18n-substrate-phase-b.md).
All agents implementing cross-platform font/emoji substrate Phase A follow
[docs/superpowers/plans/2026-06-15-font-emoji-phase-a.md](docs/superpowers/plans/2026-06-15-font-emoji-phase-a.md)
before any Phase B font asset or CSS wiring.
Phase B GUI/browser-headless work for the same issue follows
[docs/superpowers/plans/2026-06-15-font-emoji-phase-b-gui.md](docs/superpowers/plans/2026-06-15-font-emoji-phase-b-gui.md).
All agents implementing timeline navigation aids for issue #41 follow
[docs/superpowers/plans/2026-06-16-timeline-navigation-phase-a.md](docs/superpowers/plans/2026-06-16-timeline-navigation-phase-a.md)
for Phase A Rust/headless work before any Phase B GUI pills, date picker, or
scroll wiring.

## Codex Diff Review Recipe

The preferred external auditor is OpenAI `codex` (the `codex` CLI). For
substantial changes authored by non-frontier agents, run a diff review with the
command below.

Generate the diff and pipe it to `codex review -`:

```bash
cd /home/shinaoka/projects/Matrix/matrix-desktop
git diff <base-commit-sha>..HEAD > /tmp/review.diff
codex review - < /tmp/review.diff
```

Codex's Linux sandbox may fail to run `git diff` itself, so generate the diff
in the parent shell and feed it through stdin. Use `-` as the prompt argument
to read from stdin.

Add custom instructions by including them in the prompt before the diff. Write
the prompt to a file and concatenate:

```bash
cat > /tmp/review-prompt.txt <<'EOF'
Review this diff against REPOSITORY_RULES.md, docs/architecture/overview.md,
docs/architecture/state-machine.md (if reducers changed),
docs/policies/engineering-rules.md, AGENTS.md, and the relevant dated plan.
Prioritize, in order: repository-rule consistency, Rust/Tauri best practices,
security/privacy risks, then contract correctness.
Propose canon amendments when a finding is caused by a rule gap.
Keep the review private-data-free.
EOF
cat /tmp/review-prompt.txt /tmp/review.diff | codex review -
```

Run long reviews in the background:

```bash
cat /tmp/review-prompt.txt /tmp/review.diff | codex review - > /tmp/codex-review.txt 2>&1
```

Prompt contents to include:

- Ask for consistency with `REPOSITORY_RULES.md`,
  `docs/architecture/overview.md`, `docs/architecture/state-machine.md` when
  reducers change, `docs/policies/engineering-rules.md`, `AGENTS.md`, and the
  relevant dated implementation plan.
- Ask the auditor to prioritize, in order: repository-rule consistency,
  Rust/Tauri best practices, security/privacy risks, then contract
  correctness.
- Ask the auditor to propose canon amendments when a finding is caused by a
  rule gap rather than only patching the immediate code.
- Remind the auditor to keep the review private-data-free.

Important review-scope notes:

- Include `Cargo.toml` and `src/lib.rs` in the review input when the change
  adds feature gates, changes module visibility, or exposes test-only APIs.
  A diff that omits these files may produce false-positive reports about
  missing feature declarations or visibility issues.
- Include untracked new files explicitly; `git diff` alone is empty for them.
- Keep prompts private-data-free: use only synthetic fixture data, never real
  account credentials, room/event IDs, message bodies, raw SDK errors, or
  local paths.

External review findings are suggestions to verify, not automatic orders. The
main agent decides whether to adopt, escalate, or defer each proposal.

## Cost-Controlled Agent Delegation

- Use cheaper implementation agents only for bounded, low-ambiguity work:
  source search, issue inventory, single-file tests, small module-local Rust
  patches, docs consistency checks, and narrow diff reviews. Prompts must name
  the issue, allowed files, forbidden shared files, expected verification
  command, and the exact output format.
- Main GPT agents own cross-boundary design, state-machine boundary decisions,
  shared enums/DTOs, Tauri/TypeScript wire contracts, `App.tsx`,
  `TimelineView.tsx`, `styles.css`, canon docs, commits, issue comments, and
  close decisions. Cheap-agent output is a draft to verify, not accepted
  evidence by itself.
- Do not let two agents edit shared hot files concurrently. Treat
  `crates/koushi-state/src/{state.rs,action.rs,reducer.rs}`,
  `crates/koushi-core/src/{command.rs,event.rs,runtime.rs}`,
  `apps/desktop/src-tauri/src/{dto.rs,commands.rs}`,
  `apps/desktop/src/{App.tsx,components/TimelineView.tsx,i18n/messages.ts,styles.css}`,
  browser-headless specs, and Linux GUI QA scripts as main-agent integration
  points unless the task explicitly grants a narrow patch.
- Before running an expensive Linux/macOS/Windows GUI lane as a debugger, add a
  cheap private-data-free diagnostic token or title state for the missing
  product transition, then run focused Rust/Tauri/browser checks. Full native
  GUI lanes are final evidence for an issue, not the first place to discover
  command routing failures.
- Review prompts for cheap agents must ask for consistency with
  `REPOSITORY_RULES.md`, `docs/architecture/overview.md`,
  `docs/architecture/state-machine.md` when reducers or state machines change,
  `docs/policies/engineering-rules.md`, this file, and the relevant dated
  implementation plan. A silent, timed-out, or budget-exceeded cheap-agent run
  is not review evidence.

## Out of Scope (deferred)

Real-time and recorded audio/video are deferred for now and are intentionally
absent from the product roadmap:

- Voice / video calls — MatrixRTC / Element Call (MSC4143, MSC3401), including
  1:1 and group calling.
- Voice messages — recorded audio clips with waveform record/playback UI
  (MSC3245).

This is a conscious "not yet" decision, not a permanent exclusion; revisit
before GA. Do not open feature issues for these without re-deciding scope here.

## Core Batch A DTO Mirrors

- When `AppState` gains a Core Batch A field, update the hand-maintained Tauri
  `FrontendAppState` DTO, TypeScript `AppState`, browser fake snapshots, app
  harness snapshots, and Tauri IPC mock snapshots in the same change. The real
  WebView consumes the Tauri DTO, while headless tests often consume the
  TypeScript fakes; updating only one side can leave a green browser tier and a
  crashing Tauri lane.
- Focused checks for the shared skeleton are
  `cargo test -p koushi-state --test core_batch_a_state`,
  `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml core_event_wire_format_matches_checked_in_contract_artifact`,
  and `npm --prefix apps/desktop run typecheck`.

## State Transport Phase Notes

- The #111 state-transport architecture is Rust-owned incremental slice deltas
  plus a selector-subscribed WebView projection cache. React may cache and
  subscribe to Rust snapshots for rendering, but it must not mutate product
  state, synthesize Matrix semantics, or repair command results locally.
- During Phase 1, existing full snapshots still enter the WebView. Apply them
  through the projection store by value-comparing top-level
  `DesktopSnapshot`/`AppState` slices and preserving references for unchanged
  `domain`, `ui`, `sidebar`, timeline, and thread data. Hot derived arrays such
  as mention candidates and forward destinations must be memoized from Rust DTO
  input references.
- When Phase 2 adds `CoreEvent::StateDelta` / changed-slice DTOs, update
  `apps/desktop/src-tauri/src/dto.rs`, `apps/desktop/src/domain/types.ts`,
  `coreEvents.generated.json`, browser fakes, Tauri IPC mock, app harness
  snapshots, and serialization-contract tests in the same change. Full
  snapshots are then initial/reset fallback only, with generation-gap recovery.
- Tauri Channels are high-frequency only and measurement-gated. Keep crawler,
  typing, receipt, and presence semantics Rust-owned; a Channel transports
  Rust projections, not React-local state.

## Japanese / CJK Phase A Notes

- Japanese/CJK product semantics stay Rust-owned. React may render the
  `ja` catalog and Rust-owned ordering/highlight data, but it must not compute
  CJK normalization, collation, query folding, or highlight repair locally.
- CJK GUI text fitting is a presentation contract, not a product-semantic
  workaround. Long room names, member/sender names, message bodies, thread
  labels, and search snippets must keep Rust-owned text/order unchanged while
  CSS supplies `line-break: strict`, `word-break: normal`, `hyphens: none`,
  logical spacing, and width-aware ellipsis/wrapping as appropriate.
- Search/review paths for this area are:
  `apps/desktop/src/i18n/messages.ts`,
  `apps/desktop/src/i18n/messages.test.ts`,
  `apps/desktop/src/styles.css`,
  `apps/desktop/e2e/basic-operations.spec.ts`,
  `crates/koushi-state/src/locale_profile.rs`,
  `crates/koushi-state/tests/locale_display_profile.rs`,
  `crates/koushi-search/src/document.rs`,
  `crates/koushi-search/src/verify.rs`,
  `crates/koushi-search/tests/search_adapter.rs`, and
  `crates/koushi-core/src/search.rs`.
- Fast focused checks are:
  `npm --prefix apps/desktop run test -- --run src/i18n/messages.test.ts`,
  `npm --prefix apps/desktop exec -- playwright test e2e/basic-operations.spec.ts -g "Japanese locale renders shell labels and CJK text without clipping|thread and edit composers composing Enter" --workers=1`,
  `cargo test -p koushi-search --test search_adapter`,
  `cargo test -p koushi-state --test locale_display_profile`, and
  `npm --prefix apps/desktop run typecheck`.

## Credential Health QA

- Local-encryption / credential-store health is Rust-owned
  `AppState.local_encryption`; GUI code must dispatch typed
  `probe_local_encryption_health` / `reset_local_data` commands and render the
  snapshot, not infer OS/keyring semantics.
- Browser-headless Settings/Security GUI tests seed Rust-shaped
  `AppState.local_encryption` snapshots and Linux/macOS/Windows platform
  profiles. React may render the coarse status, show recovery/reset
  affordances, and dispatch `probe_local_encryption_health` /
  `reset_local_data`; it must not read OS/keyring errors, infer fail-open
  behavior, locally change health after a click, or clean stores through any
  React-local logout path. `reset_local_data` is owned by Rust
  `AccountActor`/`StoreActor`, clears current-account local persistence, and
  returns the app to a local signed-out snapshot.
- Fast Tier 1 checks are:
  `cargo test -p koushi-state --test local_encryption_state`,
  `cargo test -p koushi-key credential_backend`,
  `cargo test -p koushi-core store_actor_probe_maps_credential_backend_health_without_raw_errors`,
  and
  `cargo test -p koushi-core reset_local_data_clears_current_account_persistence_and_signs_out_locally`.
- The local headless proof is
  `PATH=/tmp/koushi-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:headless-local -- --server=conduit --scenario=credential_health --core --core-backend=both --timeout-ms=240000`.
  It runs under the debug/test file credential-store guard and must refuse to
  touch the OS keychain.
- macOS Tier 2 real-Keychain proof is opt-in. The previous manual GitHub
  Actions lane is disabled for now: the preserved recipe lives at
  `.github/workflows.disabled/macos-keychain-tier2.yml`, and GitHub also has the
  workflow disabled manually. Do not run `gh workflow run
  macos-keychain-tier2.yml` until that file is deliberately moved back under
  `.github/workflows/` and re-enabled. Use a manual macOS session instead.
  Consent dialogs, Touch ID, locked login-keychain UX, and signed-build ACL
  behavior remain attended-only. Keep any future workflow key-crate-only: it
  copies `crates/koushi-key` to `$RUNNER_TEMP` and runs
  `cargo test --manifest-path` there, so it must not require the private
  vendored Matrix SDK submodule. For a manual macOS session without an
  initialized vendor submodule, use the same temp-copy pattern before setting
  `KOUSHI_MACOS_KEYCHAIN_QA=1`. The test treats
  `security set-key-partition-list` as best-effort on hosted runners; the
  pass/fail proof is the real backend set/get/delete plus missing-credential
  mapping after delete. The test temporarily makes the throwaway keychain the
  user default keychain and restores the prior default in a guard, because the
  macOS `keyring` backend writes generic passwords through the default
  keychain. Locked-keychain reads on hosted runners can block on native
  authentication UI, so locked login-keychain prompt behavior remains Tier 3
  attended evidence.

## Native Attention QA

- Notification, badge, sound, tray, and activation decisions are Rust-owned
  `AppState.native_attention` projections. GUI/native adapter code must render
  or dispatch from the snapshot/capability DTOs; it must not invent notification
  candidates, badge counts, dedupe, or suppression semantics.
- Notification preferences are Rust-owned `SettingsValues.notifications`
  product state and must persist through the settings store with legacy JSON
  backfill. React settings UI may dispatch typed `SettingsPatch.notifications`
  updates, but it must not keep independent local notification policy state.
  Browser-headless notification settings tests must click the visible switches
  and assert the resulting `update_settings` payload; the UI must reflect
  changed switch state only after the Rust-shaped settings snapshot updates.
- Display preferences such as code-block line wrapping are Rust-owned
  `SettingsValues.display` product state and must persist through the same
  settings store with legacy JSON backfill. React may map
  `code_block_wrap` to CSS in Phase B and may omit timeline rows only from the
  Rust-projected `TimelineItem.is_hidden` flag. It must not keep a separate
  local display-policy store, derive redacted visibility from React settings
  state, or repair switch state after dispatch.
- Received Matrix `formatted_body` is a Rust-owned security projection. Core
  sanitizes Matrix HTML into `TimelineItem.formatted` before it crosses the
  WebView boundary, including plain-text and code-block metadata. React must
  render only that Rust-owned DTO; it must never render unsanitized server HTML
  or own ad hoc Matrix HTML sanitizer policy.
- The TimelineView formatted-message renderer is presentation-only. It may map
  the Rust-owned sanitized HTML/code-block DTO into React nodes and copy-code
  controls, but any tag/attribute safety decision belongs in Rust. When adding
  supported tags, extend the Rust projection tests first, then the React
  renderer and browser-headless checks.
- React attention helpers may only map `snapshot.state.native_attention` to
  window title, badge, and native adapter payloads. They must not aggregate
  `rooms`, diff previous room snapshots, or infer focused-room/muted/duplicate
  notification behavior locally.
- Keep persistent and transient native attention effects separate. Window
  title, badge count, Windows overlay, tray count, and zero-badge clearing are
  snapshot-state mappings. Sound and activation are candidate-scoped transient
  effects and may run only from a Rust-owned notification candidate plus the
  Rust-owned capability DTO; do not trigger them from every unread/badge
  snapshot refresh.
- Passive native notification dispatch checks the current OS permission state
  only. It must not call permission-prompt APIs; permission prompts belong to an
  explicit user/onboarding action.
- Native notification clearing is adapter-only and best-effort. When Rust-owned
  attention state drops the badge count to zero (including logout/account clear),
  React may call the native transport clear hook, but it must not mutate Matrix
  state or synthesize read/focus semantics locally.
- Focused acceptance checks for adapter failure/clear behavior are
  `npm --prefix apps/desktop run test -- src/domain/desktopAttention.test.ts`
  and
  `cargo test -p koushi-state --test session_state logout_clears_native_attention_state_and_notifies_ui`.
- Native attention platform capability profiles are Rust-owned and resolved from
  the shared `DisplayPlatform` model before reaching React. Add macOS/Linux/Win
  capability differences there; do not scatter platform branches through React
  components or notification helpers.
- Windows taskbar overlay support is modeled as the Rust-owned
  `NativeAttentionCapabilities.overlay_icon` field. React adapter code may call
  `setOverlayIcon` only from that DTO capability, never from direct OS sniffing.
- Space rail attention badges are Rust-owned `SidebarModel.space_rail` counts
  produced by `compose_sidebar`; `WorkspaceRail` may render the snapshot
  attributes but must not recompute child-room unread/highlight state. Timeline
  thread chips render the Rust-projected row `thread_summary` DTO. Pane-level
  thread attention is Rust-owned `AppState.thread_attention`; React may render
  the Tauri/TypeScript DTO but must not scan visible thread rows or row chips to
  derive indicator counts. The current core producer counts only remote live
  thread timeline message diffs; backfill/prepend diffs and the current user's
  own messages are ignored.
- GUI thread indicators, including the Threads nav badge/markers, render only
  `AppState.thread_attention.notification_count`, `highlight_count`, and
  `live_event_marker_count`. Do not derive them from room-list totals,
  `TimelineItem.thread_summary`, or visible thread rows.
- Notification sound policy is Rust-owned `SettingsValues.notifications.sound`.
  React may pass that DTO value into native adapter routing so sound is skipped,
  but it must not create an independent notification preference or mutate
  native attention state locally.
- Candidate projection uses private-data-minimized room labels and counts only.
  It must not expose message bodies, sender IDs, room IDs, event IDs,
  transaction IDs, raw SDK errors, or tokens in snapshots, logs, Debug output,
  QA artifacts, or issue evidence.
- Fast checks are:
  `cargo test -p koushi-state --test attention_surface`,
  `cargo test -p koushi-sdk --test attention_surface`, and
  `cargo test -p koushi-core --features qa-bin --bin headless-core-qa`.
- The local headless proof is
  `PATH=/tmp/koushi-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:headless-local -- --server=conduit --scenario=native_attention --core --core-backend=both --timeout-ms=240000`.
  It prints only `notification_candidate=ok`, `badge_state=ok`,
  `suppress_focus=ok`, and `clear_badge=ok`.

## Public Directory Phase A Notes

- Public directory semantics are Rust-owned. `AppState.directory.query` and
  `AppState.directory.join` are separate state machines; React must render
  those DTOs and dispatch typed `query_directory` / `join_directory_room`
  commands only. Do not recreate query, pagination, join success, or failure
  state in React.
- Directory join is alias-based. The SDK wrapper rejects bare room IDs for the
  directory flow; GUI code should pass the canonical alias and optional server
  hint from the Rust directory result.
- When adding or changing public directory fields, update
  `apps/desktop/src/domain/types.ts`,
  `apps/desktop/src/domain/coreEvents.ts`,
  `apps/desktop/src/domain/coreEvents.generated.json`,
  Tauri DTO tests, browser fake snapshots, app harness snapshots, and the Tauri
  IPC mock in the same change.
- The local core QA `directory` scenario proves public directory query and
  alias join through token-only stdout (`directory_query=ok`,
  `directory_join=ok`). Do not print room IDs, aliases, server names, query
  text, pagination tokens, or raw SDK errors for this stage.

## Message Interactions Phase A Notes

- `TimelineItem.reply_quote` is a Rust-owned projection. React renders the
  `ReplyQuoteState` and optional preview only; it must not look up reply
  bodies, classify redactions, or patch quote state after a send.
- `TimelineItem.actions` is a Rust-owned action-affordance projection. React
  may render/copy only the DTO-provided body/permalink affordances; it must not
  build `matrix.to` permalinks, infer copy/forward/source eligibility from
  event ids, body/media fields, or redaction flags, or synthesize message-source
  / forward semantics locally.
- `TimelineCommand::LoadMessageSource` and `TimelineCommand::ForwardMessage`
  are the typed Phase A path for view-source/forward GUI work. The source DTO is
  a safe Rust projection, not raw Matrix JSON. Forwarding sends the Rust-
  projected visible body only; media-only rows must remain non-forwardable until
  a dedicated media-forward contract exists.
- Phase B message-action menus render only `TimelineItem.actions` affordances.
  Copy uses the Rust-projected row body or Rust-built permalink only; view
  source dispatches `load_message_source` and waits for
  `MessageSourceLoaded` before showing the source dialog; forward dispatches
  `forward_message` with Rust-snapshot room destinations and never copies the
  message body in React.
- Tauri production timelines render from the CoreEvent-backed `TimelineView`
  store, not `AppState.timeline`. A local GUI lane that needs a real row/action
  must wait for DOM state such as `.message`, `data-event-id`, or
  `button[aria-label="Message actions"]`; `timeline_items=0` in the QA title
  can be normal because that token comes from the snapshot DTO.
- `AppState.room_interactions` is the Rust-owned source of truth for
  `pinned_events` and `pin_operation`. GUI code dispatches typed `pin_event` /
  `unpin_event` commands and waits for Rust-shaped snapshots/events instead of
  mutating local pin lists.
- Recoverable pin/unpin failures must remain retryable in the reducer. Do not
  clear failed pin state from React; a new typed request transitions the Rust
  state from `Failed` to `Pending`.
- Pin/unpin command success settles the Rust pending state before the follow-up
  pinned-event reload. A reload failure may emit a coarse operation failure, but
  it must not leave the GUI stuck in `Pending`.
- Browser fakes must enforce the same known-room guard as the Rust reducer and
  `RoomActor`; do not let tests create `room_interactions[roomId]` for a room
  absent from `state.rooms`.
- When changing `TimelineItem.reply_quote`, `TimelineItem.actions`,
  `TimelineMessageSource`, message forward/source command/event variants,
  `PinnedEvent`, `RoomInteractionState`, or pin/unpin command/event variants,
  update the Tauri DTO, TypeScript domain types, `coreEvents.generated.json`,
  browser fake, app/IPC harness snapshots, and serialization-contract tests in
  the same change.
- The local core `reply` QA stage uses token-only evidence:
  `reply_quote=ok`, `pin_event=ok`, `pinned_state=ok`, and `unpin_event=ok`.
  Do not print Matrix room IDs, event IDs, sender IDs, message bodies, or raw
  SDK errors for this stage. Message-action/permalink evidence must also stay
  token-only and must not print generated permalinks.

## User Profiles Phase Notes

- Own-profile state, per-user profile cache, room avatars, and space avatars
  are Rust-owned DTOs. React renders them and dispatches `set_display_name` /
  `set_avatar`; do not add React-local profile success/failure semantics.
- Personal local user aliases are also Rust-owned profile state. Keep alias
  set/clear/list, persistence to `app.koushi.local_aliases`, display-name
  resolution, and pending/failure state in Rust; React may render the returned
  labels and dispatch typed commands only. The SDK uses
  `app.koushi.local_aliases` only; old Kagome-era account-data migration is not
  required for this project state.
- `UserProfile.display_label`, `UserProfile.original_display_label`, and
  `UserProfile.mention_search_terms` are the Rust-owned person/mention
  projection. `display_label` may contain the local alias; `original_display_label`
  is the alias-free upstream/own-profile/MXID context value. GUI mention
  suggestions/highlighting and profile/tooltips must use the projected fields
  instead of recomputing alias precedence or stripping aliases in React.
- Timeline sender display is Rust-projected. `sender`, reply quote `sender`,
  and thread-summary `latest_sender` remain raw identity fields; normal
  TimelineView display must use `sender_label`, `reply_quote.sender_label`, and
  `thread_summary.latest_sender_label` when present. Do not repair missing
  labels in React by joining sender ids to `local_aliases`.
- Existing timeline rows are relabeled through the keyless Rust
  `TimelineEvent::DisplayLabelsUpdated` patch stream after profile/alias
  changes. Frontend stores may match raw identity fields and apply the supplied
  labels across loaded timelines, but React must not resolve alias precedence or
  synthesize fallback labels. When clearing an alias, keep the target user id in
  the Rust emission even if the user is absent from `profile.users`.
- Room-scoped member labels are Rust-projected too:
  `RoomMemberSummary.display_label` is resolved from
  `ProfileState.local_aliases`, nonblank room-scoped upstream `display_name`,
  profile cache / own-profile fallback, and finally MXID when room settings load,
  room settings update, or profile/alias state changes.
  `RoomMemberSummary.original_display_label` carries the alias-free context
  label. `display_name` remains the upstream/original raw value. React member
  lists, sort order, action labels, and original-name affordances must consume
  these projected fields and must not join `settings.members` with
  `local_aliases` or the global profile cache.
- Local alias GUI affordances dispatch only the typed
  `set_local_user_alias(user_id, alias)` account command. React may own dialog
  visibility and input draft text, including trimming empty input to a clear,
  but it must not update member rows, DM titles, timeline labels, receipts, or
  mention candidates locally; those must change only after the returned
  Rust-shaped snapshot or timeline label event changes. Browser-headless tests
  should assert the typed command arguments and then assert Rust-projected
  `display_label` / `original_display_label` rendering.
- Room summaries use the same Rust-owned display projection:
  `RoomSummary.display_label` is the sidebar/header/search/forward/space-child
  display value, while `RoomSummary.original_display_label` is the alias-free
  room/DM context value. `display_name` remains the upstream/original room name.
  For one-to-one DM rooms, `dm_user_ids` carries the target identity and labels
  resolve through local alias, nonblank upstream room name, profile/own-profile,
  then MXID. Non-DM rooms use trimmed upstream `display_name`, then `room_id`.
  `display_label`/`original_display_label` are caller-owned room/user data, not
  i18n catalog prose; do not invent generic English fallbacks such as `Member`,
  and do not infer the DM target from a room title in React.
- Read-receipt readers carry both `display_name` (the Rust-projected visible
  label, despite the legacy field name) and `original_display_label` for
  alias-free hover/profile context. React must not recover original names by
  looking up profiles or stripping a local alias.
- Native attention uses `RoomSummary.display_label` for its safe room label, but
  serialized candidates still must not carry room IDs, sender IDs, event IDs, or
  message bodies. Profile/alias relabeling of an existing candidate is a Rust
  reducer projection over `state.rooms`, not a React notification-policy repair.
- Local aliases are private "only I see this" data. Do not print alias user ids
  or alias text in normal Debug, QA titles/logs, screenshots, issue comments, or
  docs examples. `ProfileState` Debug should expose only profile/avatar presence
  and counts; SDK alias DTO Debug should expose counts only.
- When adding GUI labels for alias/profile affordances, update the
  `MessageId` union, English and Japanese catalogs, and `messages.test.ts`
  coverage together; adding only the English catalog makes runtime `t(...)`
  calls fail before the typecheck catches the missing key.
- `SetAvatar` may carry image bytes only through the typed command boundary.
  Debug output, QA logs, screenshots, issue comments, and docs examples must not
  contain real avatar bytes, real avatar MXC URIs, local thumbnail paths, or raw
  SDK errors.
- `AvatarImage.mxc_uri` is metadata, not a render URL. GUI code renders an
  `<img>` only for `AvatarThumbnailState::Ready.source_url`; otherwise it uses
  the colored-initial fallback. This keeps the current #15 media contract intact
  because timeline `download_media` emits byte counts only.
- When adding or changing `AppState.profile` or avatar fields, update the
  hand-maintained Tauri DTO (`apps/desktop/src-tauri/src/dto.rs`), TypeScript
  domain types, browser fake API, Tauri IPC mock, app harness snapshots, and DTO
  serialization-contract tests in the same change. Browser fakes do not inherit
  Rust snapshot fields automatically.
- Profile update completion settles a user-visible pending state. Actor code
  must deliver `ProfileUpdateSucceeded` / `ProfileUpdateFailed` reliably via
  the action channel, not as a best-effort notification that can leave settings
  controls stuck in a saving state.

## Room Tags Phase A Notes

- `RoomSummary.tags` is the Rust-owned source of truth for Matrix `m.tag`
  favourite and low-priority state. React may render tag affordances and dispatch
  `set_room_tag` / `remove_room_tag`, but it must not keep local tag membership
  or repair room-list sections after the fact.
- Favourite and low-priority are mutually exclusive in
  `koushi-state`. Keep this reducer rule in sync with the SDK wrappers:
  use `koushi-sdk`'s `set_room_tag` / `remove_room_tag`, which delegate
  to `Room::set_is_favourite` and `Room::set_is_low_priority`; do not patch the
  vendored SDK for this behavior.
- Tag command success must not immediately request a room-list refresh. The SDK
  tag calls send account-data changes to the homeserver, and the local SDK room
  snapshot can remain stale until the next sync. Project the successful command
  through `RoomTagSet` / `RoomTagRemoved` reducer actions, then let the next
  sync snapshot become canonical.
- When adding fields to `RoomSummary`, update every projection and fake snapshot
  in the same change: `koushi-core::room::normalize_rooms`,
  `koushi-state::sidebar::RoomListItem`, `apps/desktop/src-tauri/src/dto.rs`,
  `apps/desktop/src/domain/types.ts`, `browserFakeApi.ts`,
  `appHarnessMain.tsx`, and any Rust/TS fixtures that construct `RoomSummary`.
- Sidebar shell affordances (section counts, unread badges, mention dots) render
  `SidebarModel` fields such as `unread_count` / `highlight_count`. When a
  sidebar projection field changes, update Rust `compose_sidebar`, the Tauri DTO
  serialization-contract test, `types.ts`, browser fake snapshots,
  `tauriIpcMock`, app harness snapshots, and browser-headless shell tests
  together.
- New tag command/event variants must keep all three IPC surfaces in sync:
  `serialize_core_event`, `apps/desktop/src/domain/coreEvents.ts`, and
  `apps/desktop/src/domain/coreEvents.generated.json`. Verify with
  `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml core_event_wire_format_matches_checked_in_contract_artifact`.
- Phase B room-list sections (Favourites / People / Rooms / Low priority) must
  derive from Rust snapshots (`RoomSummary.tags` + `is_dm`). Do not introduce
  React-only section membership while wiring context menus or browser-headless
  tests.
- Phase B room-tag GUI tests should stub `set_room_tag` / `remove_room_tag`
  to return the current snapshot first, assert the row does not move
  immediately, then push a Rust-shaped snapshot with updated `RoomSummary.tags`
  / sidebar room tags and assert the section movement. This catches accidental
  React-local room-list repair.

## Outbound Send Queue Notes

- Retry/cancel is driven by SDK `SendHandle`, not by a direct
  `RoomSendQueue::retry(transaction_id)` API. `TimelineActor` must keep a
  transaction-id keyed handle registry initialized from
  `RoomSendQueue::subscribe()` local echoes and updated by
  `RoomSendQueueUpdate::NewLocalEvent`.
- Recoverable SDK send errors disable the room send queue. `RetrySend` must
  call `room.send_queue().set_enabled(true)` before `SendHandle::unwedge()`;
  successful `CancelSend` must also re-enable the room queue after
  `SendHandle::abort()` so successors are not stranded behind a removed failed
  item.
- `TimelineItem.send_state` is a Rust-owned DTO projection. React may render it
  and dispatch `retry_send` / `cancel_send`, but must not infer send legality
  from `TimelineItemId::Transaction` or repair queue state locally.
- `TimelineItemId::Transaction` is a stable identity for local echoes, not a
  UI state. A transaction row without `send_state` must not be labeled unsent;
  failed/sending/cancelled affordances come only from `send_state`.
- Phase B send-queue GUI tests should seed Rust-shaped CoreEvent timeline items
  in `appHarnessMain.tsx` / `basic-operations.spec.ts`, click the visible
  controls, and then push a CoreEvent diff to prove the UI reflects Rust-owned
  state changes. Do not update React state directly after `retry_send` or
  `cancel_send`.
- `RoomSendQueueUpdate::SendError` carries raw SDK errors. Project only coarse
  recoverable/unrecoverable status into DTOs and QA tokens; do not print raw SDK
  errors, transaction ids, Matrix ids, or message bodies in QA output.
- The `headless-core-qa` `send_queue` scenario injects offline failure through
  a stdlib TCP proxy inside the Rust QA binary and must be run with
  `--features qa-bin`; plain `cargo test` does not compile that binary. Verify
  both SyncService and LegacySync legs when changing retry/cancel semantics.
- New timeline item DTO fields must keep
  `apps/desktop/src/domain/coreEvents.ts`,
  `apps/desktop/src/domain/coreEvents.generated.json`, and
  `apps/desktop/src-tauri/src/lib.rs`'s core-event wire contract test in sync.

## Timeline Navigation Phase A/B Notes

- Timeline navigation semantics stay in Rust. React may report viewport facts
  through `observe_timeline_viewport` (`first_visible_event_id`,
  `last_visible_event_id`, `at_bottom`) and may scroll to returned anchors, but
  it must not compute read-marker placement, first-unread targets, unread
  counts, or jump-to-bottom counts.
- `TimelineActor` emits `TimelineEvent::NavigationUpdated` from the current
  projected item order and fully-read marker. Diff-driven navigation updates
  must be emitted after `ItemsUpdated` so GUI rows exist before a Phase B scroll
  action references them.
- Jump-to-date uses `open_timeline_at_timestamp`, which routes through
  `AppCommand::OpenTimelineAtTimestamp` and the Matrix `timestamp_to_event`
  endpoint in Rust before reusing focused context. React must not call raw
  Matrix APIs for date jumps.
- The local core timeline proof now includes token-only `timeline_nav=ok`.
  Keep this private-data-free: no room ids, event ids, user ids, message
  bodies, timestamps, or raw SDK errors.
- Phase B `TimelineView` renders first-unread/bottom pills only from
  `TimelineEvent::NavigationUpdated`. The date picker dispatches
  `open_timeline_at_timestamp`; it must not resolve event IDs in React.
- The existing read-receipt/fully-read auto dispatch is constrained to the
  bottom viewport. If the viewport is not at bottom, React reports only
  `observe_timeline_viewport` facts so Rust can keep unread navigation
  projection stable.
- Linux virtual-display coverage is `--scenario=local-timeline-navigation`.
  It seeds a scrollable local timeline, uses a helper user for unread messages,
  clicks the first-unread and bottom pills, then drives jump-to-date into
  focused context. It prints `gui_local_timeline_unread_jump=ok`,
  `gui_local_timeline_bottom_jump=ok`, and
  `gui_local_timeline_date_jump=ok`:
  `PATH=/tmp/koushi-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-timeline-navigation --server=conduit --skip-build --artifact-dir=artifacts/linux-gui-local-timeline-navigation-fast --timeout-ms=180000`

## Live Signals Phase A Notes

- `AppState.live_signals` is the Rust-owned source of truth for read receipts,
  fully-read markers, typing users, and presence. React may render it and
  dispatch typed commands only; do not add React-local receipt, marker, typing,
  or presence semantics.
- Read-receipt reader avatars are also Rust-owned live-signal projection data:
  reducers resolve reader display labels and avatar DTOs from profile state,
  dedupe by reader using the newest timestamp, order readers most-recent-first,
  cap the rendered readers, and expose `overflow_count`. `TimelineView` renders
  that DTO and may own only tooltip visibility through DOM/CSS; do not join
  receipts with `profile.users` in React.
- Timeline live-signal commands route through `TimelineCommand` and the
  subscribed `TimelineActor`: `SendReadReceipt`, `SetFullyRead`, and
  `SetTyping`. Account presence routes through `AccountCommand::SetPresence`.
  Keep SDK handles and sync policy in Rust actors.
- The current Phase A presence implementation records and emits the requested
  Rust-owned presence state. Full network presence propagation remains a sync
  backend decision because the legacy SDK path exposes `SyncSettings` presence
  while the current `SyncService` builder does not expose a direct setter.
- The Tauri snapshot is a hand-maintained DTO. When `AppState.live_signals` or
  another `AppState` field is added, update `apps/desktop/src-tauri/src/dto.rs`,
  `apps/desktop/src/domain/types.ts`, `browserFakeApi`, `tauriIpcMock`, app
  harness snapshots, and DTO serialization-contract tests in the same change.
  Headless browser mocks do not inherit Rust fields automatically.
- New `CoreEvent` variants need the Rust wire-contract test, generated
  `apps/desktop/src/domain/coreEvents.generated.json`, and
  `apps/desktop/src/domain/coreEvents.ts` updated together. Do not hand-write
  a TypeScript shape that is not proven by the Rust contract artifact.
- The local core QA `live_signals` scenario is token-only:
  `read_receipt=ok`, `fully_read=ok`, `typing=ok`, `presence=ok`,
  `live_signals=ok`. Do not print Matrix room IDs, event IDs, user IDs, message
  bodies, or raw SDK errors for this stage.
- If the `live_signals` scenario reaches `fully_read=ok` and then times out at
  typing on a probed SyncService local homeserver leg, first verify the legacy
  backend. In this environment, legacy receives the typing notification
  continuously, while the SyncService local leg needs a bounded debug/test
  `SyncOnce` on the observer account after `SetTyping` is acknowledged to wake
  the same Rust-owned typing observer. Do not replace this with React polling or
  local UI timers.

## Activity Phase A Notes

- `AppState.activity` is the Rust-owned source of truth for account-wide
  Recent/Unread Activity. React may render `ActivityState`, switch tabs through
  `set_activity_tab`, request pagination, open focused context from the row's
  event reference, and dispatch `mark_activity_read`; it must not sort,
  synthesize unread membership, clear rows locally, or derive account-wide
  Activity from `TimelineView` DOM state.
- Activity rows are observed by room `TimelineActor`s as `ActivityRowsObserved`
  and materialized by the `AppActor`'s Activity projection cache. The projection
  fills room labels, unread flags, highlight flags, and low-priority exclusion
  from Rust-owned `AppState` facts. Keep this cache outside React and outside
  per-view browser fake state.
- Opening or paginating Activity snapshots the Rust projection into separate
  Recent and Unread streams. Viewing the Unread tab does not mark anything read.
  `MarkActivityRead` settles both room targets and the all-activity target
  through the Rust `mark_read` substate and then updates Activity streams;
  future SDK fully-read writes must stay behind the same typed command boundary.
- When adding or changing `ActivityState`, `ActivityRow`, `ActivityEvent`, or
  Activity command shapes, update the Tauri DTO, TypeScript domain types,
  checked-in CoreEvent contract artifact, browser fake, Tauri IPC mock, app
  harness snapshots, and serialization-contract tests in the same change.
- The local core QA `activity` scenario is token-only:
  `activity_recent=ok`, `activity_unread=ok`, and `activity_markread=ok`. Do not
  print Matrix room IDs, event IDs, sender IDs, message bodies, pagination
  tokens, or raw SDK errors for this stage.
- Browser-headless Activity GUI tests should seed Rust-shaped
  `AppState.activity` snapshots and assert that rows stay visible after
  `mark_activity_read` until a later snapshot removes them. Do not make React
  sort rows, infer low-priority exclusion, auto-clear Unread on tab view, or
  repair mark-read results locally.
- The Linux virtual-display `--scenario=local-activity` lane is a real Tauri
  smoke for the Activity rail entry and tab switching. It intentionally leaves
  Recent/Unread row ordering, focused-context row jumps, and mark-read
  correctness to the Rust core and browser-headless gates, and prints only
  `gui_local_activity_open=ok`, `gui_local_activity_unread_tab=ok`, and
  `gui_local_activity_recent_tab=ok`.

## Live Signals Phase B Notes

- The full-app browser harness (`apps/desktop/src/test/appHarnessMain.tsx`)
  must import `../styles.css`, matching production `main.tsx`. Otherwise
  visibility/layout assertions can pass against unstyled DOM and miss real
  production CSS issues.
- GUI-only tooltips are presentation state only when their displayed text
  already comes from Rust-owned snapshots, such as `space.display_name`.
  Prefer the reusable `Tooltip` component over native `title=` for styled,
  testable tooltips: it must expose `role="tooltip"`, add
  `aria-describedby` only while open, open on hover/focus, and dismiss on
  mouse-leave/blur/Escape.
- Do not add ad hoc `px` literals in TSX for fixed GUI geometry. Repeated or
  semantic dimensions for rails, icon buttons, badges, avatars, counters, and
  tooltip placement belong in named CSS custom properties. Keeping `px` behind
  a token is acceptable for deliberately fixed-format controls; text-driven
  layout should prefer logical properties and scalable units where practical.
  Repeated Lucide icon `size` props are fixed GUI geometry too; centralize them
  in a local constant map instead of scattering numeric props through React.
- Event-driven `TimelineItemRow` uses the same `.message` grid contract as the
  legacy snapshot `MessageArticle`: direct child `.avatar`, `.message-main`,
  and row-level `.message-actions`. Keep direct-child grid placement explicit;
  pre-placing the actions without placing the main cell can push message
  content into the 44px avatar column and hide media titles.
- React may use refs only to suppress duplicate viewport-triggered command
  dispatches such as mark-read/read-receipt sends. Receipt, read-marker,
  typing, and presence values themselves remain Rust-owned
  `AppState.live_signals`.

## E2EE Trust Phase A Notes

- Device verification SDK handles are actor-private resources. Keep
  `VerificationRequest` and `SasVerification` wrapped in
  `koushi-sdk` opaque handles and store them only inside
  `AccountActor`; snapshots, Tauri DTOs, TypeScript types, and React state get
  only `VerificationFlowState` plus private-data-free SAS emoji DTOs.
- Verification progress is Rust-owned. `AccountActor` listens to SDK
  request/SAS state streams and projects `VerificationSasPresented`,
  `VerificationCompleted`, or `VerificationFailed`; GUI code must not infer
  SAS readiness, completion, or cancellation from local React state.
- SAS mismatch is not a generic UI cancel. Route it as
  `VerificationCancelReason::Mismatch` so the reducer settles
  `VerificationFlowState::Failed { kind: Mismatch }` and `AccountActor` calls
  the SDK `SasVerification::mismatch()` path. Plain user decline/cancel uses
  `VerificationCancelReason::User` and returns the reducer to `Idle`.
- Incoming verification requests are discovered by the Rust `AccountActor`
  observer, not by GUI code. Follow-up verification commands must pass the
  Rust-owned `flow_id` from `AppState`; their command `request_id` is separate
  and is used only for command submission/failure correlation.
- Incoming verification observers may report the same SDK verification flow
  more than once as sync catches up. `AccountActor` must ignore duplicate
  incoming requests with the same SDK `flow_id`; only a different active flow
  should be cancelled/rejected.
- SAS peer acceptance is driven by SDK SAS state, not by React state or the SDK
  `we_started` flag. In this wrapper, `Started` is the peer side that must call
  `accept_sas_verification`; `Created` is the local side after `start_sas` and
  must not be auto-accepted.
- In same-user two-device SAS QA, keep the request direction A2 -> A and let
  the requester A2 start SAS after A accepts. Starting SAS from the accepting
  device reproduced Tuwunel `m.key_mismatch` cancellation before emoji
  presentation, while the requester-start sequence is stable across local
  Conduit and Tuwunel.
- During the local SAS proof, do not overlap continuous SyncService delivery
  with manual `SyncOnce` nudges. Start the verification request while sync is
  running so device data is fresh, then pause both sync loops and drive SAS
  request/ready/start/key/done with bounded `SyncOnce` polling. Overlap
  reproduced pre-SAS key-mismatch flakes.
- Identity-reset auth continuation follows the same separation: GUI commands
  must use a fresh command `request_id` for submission correlation and pass the
  Rust-owned identity-reset `flow_id` from
  `AppState.e2ee_trust.identity_reset`. Do not reuse React-local pending ids or
  infer the flow from button state.
- Verification observers and SDK handles must be stopped/cancelled on logout,
  account switch, and actor shutdown before dropping the Matrix session.
- `BootstrapCrossSigning` may carry a UIAA password `AuthSecret` only inside
  the `CoreCommand::Account` command boundary. Its reducer action, effect,
  event, snapshot, logs, and `Debug` output must remain secret-free.
- `EnableKeyBackup` may carry an optional recovery passphrase `AuthSecret`
  only inside the `CoreCommand::Account` command boundary. Use it for
  passphrase-backed local proof or future product input, but never project the
  passphrase or returned recovery key into reducer state, DTOs, logs, or QA
  output.
- Secure-backup setup/passphrase-change may produce a new recovery key through
  the SDK. Do not project that key into reducer state, Tauri DTO snapshots,
  React state, logs, QA tokens, screenshots, or issue comments. Desktop
  recovery-key delivery writes through the Rust/Tauri native artifact path and
  reports only `Written`/`NotWritten` style status.
- `RestoreKeyBackup` is secret-bearing only at the `CoreCommand::Account`
  boundary. Its reducer projection, `AppEffect`, `CoreEvent`, Tauri DTO, and
  React state must never carry the recovery secret.
- `RestoreKeyBackup` must not be runtime gated to `SessionState::Ready` only.
  A newly logged-in device can become `NeedsRecovery` after sync discovers
  secret storage, and key-backup restore is the operation that gets it out of
  that state. Let `AccountActor` enforce that a store-backed Matrix session
  exists; `SignedOut` still fails as `SessionRequired`.
- The vendored SDK's backup-wide all-room-key download helper is private.
  Current Phase A restore code must use public SDK APIs only: recover/import the
  secret, then hydrate currently joined rooms with
  `Backups::download_room_keys_for_room`. Do not patch vendored SDK just to call
  `download_all_room_keys` unless that patch is separately justified and
  recorded in the upstream feedback ledger.
- Key-backup restore progress in the current public-API slice counts joined-room
  hydration attempts. Do not describe it as exhaustive backup-wide restore until
  a local homeserver QA lane proves the exact all-session behavior.
- The local core QA `e2ee_trust` scenario logs the same synthetic user into a
  second data directory/device and proves cross-signing bootstrap, encrypted
  seed-room key-backup upload, wrong-secret restore failure, successful
  passphrase restore on the second device, SAS device verification, and
  identity reset through `CoreCommand`/`CoreEvent` only. Its stdout must stay
  token-only for these checks; do not print account keys, verification target
  user/device ids, backup versions, room ids, event ids, recovery secrets, or
  raw SDK errors.
- The local headless runner registers separate synthetic users for the SDK lane
  and each core backend leg. Keep E2EE trust proofs isolated per core leg so
  unrelated smoke-test devices do not become part of the account's device graph.
- Room-list space classification can lag behind room/space create or join on
  local homeservers, especially Conduit. Headless core QA should perform a
  bounded `SyncOnce` after A creates/invites and after B joins before asserting
  `rooms` vs `spaces`; otherwise a valid space can temporarily appear as a plain
  room and make aggregate lanes flaky.
- Invite and DM membership state is Rust-owned. `AppState.invites` is projected
  by `RoomActor` from SDK invited rooms; React must render it and dispatch
  typed commands (`AcceptInvite`, `DeclineInvite`, `StartDirectMessage`) instead
  of maintaining local invite lifecycle state. In the SyncService backend, the
  live room-list entries adapter must use the non-left filter so invited-room
  diffs wake the projection loop; a joined-only filter leaves
  `invite_recv=ok` stuck with zero invites even after sync succeeds.
- The local core QA `invites_dm` scenario proves incoming room/space invite
  receipt and accept, invite decline, and DM start/invite projection through
  token-only stdout (`invite_recv=ok`, `invite_accept=ok`,
  `invite_decline=ok`, `dm_start=ok`). Do not print Matrix room IDs, user IDs,
  or raw SDK errors for this stage.
- Run the local proof with the SyncService/probed core leg while iterating:
  `npm --prefix apps/desktop run qa:headless-local -- --server=conduit --scenario=e2ee_trust --core --core-backend=probed --timeout-ms=240000`.
  The runner supports `--core-backend=legacy|both` for non-E2EE backend
  coverage, but the Phase A E2EE trust proof is the probed SyncService leg.

## E2EE Trust Phase B GUI Notes

- Trust GUI controls are transport clients only. Add Tauri commands as thin
  `CoreCommand::Account` submitters and keep SDK calls, UIAA/OAuth continuation
  handles, and verification handles inside Rust actors.
- React must render `snapshot.state.e2ee_trust` and dispatch typed API methods.
  Do not add React-local pending/success/failure state for verification,
  cross-signing, key backup, or identity reset. Button-click feedback must come
  back through the Rust-owned snapshot/event path.
- Verification and device DTOs include user/device ids for Rust correlation,
  but the GUI should not display those ids by default. Use ordinal/status labels
  (`Device 1`, `Verified`, etc.) unless a Rust-owned redacted display model is
  added. Playwright/Vitest assertions must not print verification targets,
  account keys, backup versions, recovery secrets, or raw SDK errors.
- Identity-reset password/UIAA input may exist only as transient DOM input that
  is immediately sent to Tauri. Clear the input after submit, and verify the
  mocked IPC layer records password fields as `[REDACTED]`.
- When adding trust GUI tests, update `apps/desktop/src/test/appHarnessMain.tsx`
  with Rust-shaped `e2ee_trust` fixtures and command responses. Do not test
  trust success by mutating React component state; assert the returned snapshot
  changed and the expected Tauri command name/flow id was invoked.
- All visible trust labels/status text must go through `apps/desktop/src/i18n/messages.ts`.
  SDK-provided SAS emoji descriptions are not catalog strings; render emoji
  symbols or add a Rust-owned localized DTO before showing descriptions.

## Rust-Owned Settings Notes

- Settings product state lives in `koushi-state::AppState.settings`.
  GUI work may render it and dispatch `update_settings`, but must not make
  locale, theme, font/emoji, or composer-send shortcut preferences a React or
  localStorage source of truth.
- Locale/display behavior is resolved by
  `koushi_state::resolve_locale_display_profile`. GUI components may
  consume the resulting `lang`, `dir`, catalog locale, pseudo-locale mode,
  platform, and modifier labels, but must not parse raw language tags or own
  fallback locale rules.
- `LocaleDisplayProfile` is a snapshot contract field, not a browser-only
  convenience. When it changes, update `apps/desktop/src-tauri/src/dto.rs`,
  `apps/desktop/src/domain/types.ts`, `browserFakeApi`, `tauriIpcMock`, app
  harness snapshots, and the DTO serialization-contract tests together.
- `TypographyDisplayProfile` follows the same DTO rule. It is resolved in
  Rust from `SettingsValues.typography` plus the platform profile and exposes
  only font/emoji preference and asset-status tokens. GUI code may apply those
  tokens to root attributes/CSS; it must not invent Inter/Twemoji/system
  fallback behavior per component.
- Font asset loading is Phase B. Inter and Twemoji COLR are bundled-preferred
  choices with system fallbacks, and any included font package must update
  `THIRD_PARTY_NOTICES.md` with version, local path, license, and provenance.
  The current Twemoji COLR package (`twemoji-colr-font@15.0.3`) is pinned but
  npm marks it deprecated; do not upgrade or replace it without checking the
  rendered family name, license stack (package/font/artwork), and browser
  COLR/CPAL behavior.
- Keep the root font stack as a single resolved custom property, e.g.
  `font-family: var(--font-ui)`. A 2026-06-15 Phase B attempt used
  `font-family: var(--font-ui), var(--font-emoji)` with list-valued variables;
  headless Chromium rendered the page, but Playwright `locator.click()` hung at
  the actionability "visible, enabled and stable" step for ordinary buttons.
  Fold emoji fallbacks into `--font-ui` / `--font-message` instead of chaining
  list-valued font variables at the declaration site.
- Root `lang`/`dir` and active catalog selection come from
  `snapshot.state.locale_profile`. Raw visible strings in React components
  should fail the catalog gate unless they are reviewed structured registry
  data or synthetic fixture content.
- Composer key behavior belongs to the Rust-owned resolver in
  `koushi-state`, shared by main, thread, and edit composer surfaces.
  GUI code normalizes DOM/native key input into typed resolver facts and then
  dispatches/renders the returned action.
- Composer send semantics also stay Rust-owned. `MentionIntent`,
  markdown/html formatting, `/me` slash-command emote conversion, and
  unsupported slash-command failures are derived in Rust/core before SDK send.
  React may pass typed draft/key/selection facts, but it must not synthesize
  `m.mentions`, formatted bodies, slash-command dispatch, or a local fallback
  send path when the resolver returns `noop` or `commitImeCandidate`.
  Because the resolver crosses an async IPC boundary, GUI key handlers must not
  call `preventDefault()` for `is_composing` key events; native IME commit owns
  that browser default while Rust still owns the product action (`CommitImeCandidate`).
- Main and thread composer draft survival is Rust-owned. React reads
  `snapshot.state.timeline.composer.draft` or the open thread composer, then
  dispatches `set_composer_draft` / `set_thread_composer_draft`; do not add a
  React-local per-room/per-thread draft map. The backing store is encrypted,
  debounced, and account-scoped in `koushi-core`; it is not serialized as
  a full draft map to the webview snapshot.
- Scheduled/send-later state follows the same boundary. The full queue and
  local fallback timer are Rust/core-owned; React may render only
  `snapshot.state.timeline.scheduled_sends` for the selected room and
  `scheduled_send_capability`, then dispatch typed schedule/cancel/reschedule
  commands in Phase B. MSC4140 delayed-event capability detection and
  create/cancel/reschedule requests live in `AccountActor` through SDK/Ruma
  APIs. The local fallback timer must consider only `ScheduledSendHandle::Local`
  items; server handles are owned by the homeserver and must not be fired by
  the local timer. Do not add browser timers, React-local scheduled-message
  maps, raw Matrix delayed-event calls, or logs/screenshots containing scheduled
  message bodies or server delayed-event handles.
- Phase B scheduled-send browser-headless proof drives the real Composer
  `Send later` control and scheduled-message list, records typed
  `schedule_send`, `reschedule_scheduled_send`, and `cancel_scheduled_send`
  IPC calls, and verifies rows stay visible until a later Rust-shaped snapshot
  changes `scheduled_sends`. Use:
  `cd apps/desktop && npx playwright test e2e/basic-operations.spec.ts -g "scheduled send UI"`.
- The focused local scheduled-send QA lane is:
  `PATH=/tmp/koushi-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:headless-local -- --server=conduit --scenario=scheduled_send --core --core-backend=probed --timeout-ms=240000`.
  Required private-data-free tokens are
  `scheduled_capability=local_fallback`, `scheduled_create=ok`,
  `scheduled_reschedule=ok`, `scheduled_cancel=ok`, and `scheduled_fire=ok`.
- The focused local composer QA lane is:
  `PATH=/tmp/koushi-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:headless-local -- --server=conduit --scenario=composer --core --core-backend=both --timeout-ms=240000`.
  Required private-data-free tokens are `mention_send=ok`,
  `markdown_send=ok`, `slash_command=ok`, and `ime_guard=ok`.
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

## Key Backup Restore Scope QA

- `joined_room_restore=ok` is the #30 MVP proof token for recovery secret
  import plus currently joined-room key hydration. It is not proof of
  exhaustive backup-wide restore. `KeyBackupRestoreSummary.scope` must remain
  `JoinedRooms` unless docs/policies and upstream SDK feedback record a broader
  public API or reviewed vendored patch decision.

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
- `e2e/desktop-shell-a11y.spec.ts:18` ("the three-pane shell exposes landmarks
  and reachable keyboard focus stops") is a PRE-EXISTING failure: the
  `complementary` landmark named "Context panel" is not found/visible at the
  default `/` harness state. It fails identically on a clean checkout of the
  pre-#77-83 base commit, so it is not introduced by the #77-83 dogfood work.
  There is no hosted CI, so this a11y spec had been silently red. Track a
  separate fix for the default right-panel landmark; do not treat it as a
  #77-83 regression.
- `e2e/basic-operations.spec.ts:2811` ("pin and unpin actions render the Tauri
  snapshot response without a manual state event") is flaky in the FULL parallel
  Playwright run but passes deterministically in isolation
  (`npx playwright test basic-operations.spec.ts:2811 --workers=1`). Same class
  as the reply-spec flake above: a shared Vite-harness snapshot-timing race under
  parallel-file worker contention, not a product bug. Run pinned/snapshot specs
  isolated or with `--workers=1` to confirm.
- For i18n headless tests that first push a locale/profile snapshot and then
  mutate the event-driven timeline, prefer updating the already-seeded room row
  with `ItemsUpdated.Set` at generation `1`. A one-off `InitialItems` emitted
  around the same snapshot refresh can be swallowed by harness timing and leave
  the seed row visible, even though the root `lang`/`dir` update succeeded.
- When a Playwright helper seeds event-driven timeline rows with fake
  `CoreEvent::Timeline::InitialItems`, make the helper wait until every target
  `data-item-id` is visible and fail on timeout. Do not fire a fixed number of
  events and let the test continue: full-spec runs can otherwise hide a
  dropped harness event until a later unrelated assertion times out.
- File attachment GUI tests must not open a native file dialog. Use the
  Composer's hidden `input[type=file][aria-label="Attach file input"]` and
  Playwright `setInputFiles()` with synthetic bytes. The visible button should
  be located with `getByRole("button", { name: "Attach file", exact: true })`
  because browsers expose file inputs with button semantics and the input label
  contains the button label as a prefix.
- Media caption GUI tests must assert staging, not immediate upload: selecting
  a file shows the Rust-owned Upload attachments staging dialog and must not
  invoke `upload_media` until Send. Captions are edited through the staging
  dialog (`TimelinePaneState.staged_uploads[*].caption`), not inferred from the
  Composer draft; tests should assert no separate `send_text` invocation and
  should render the Rust-owned `TimelineItem.media` row with the caption body
  below it.
- Transaction timeline rows use `timelineItemDomId`, so local echoes render
  with `data-item-id="txn:<transaction_id>"`. Headless media-progress specs
  should target that canonical id instead of the raw transaction id.
- Media GUI rendering is DTO-only. React may display `TimelineItem.media`
  filename/mimetype/size/dimensions/encrypted flag and
  `MediaUploadProgress`, but it must not parse Matrix event content, render MXC
  URIs, store downloaded bytes, or synthesize upload/download lifecycle state.
- Image upload compression keeps the same split: Rust owns
  `SettingsValues.media.image_upload_compression`, policy
  threshold/target/quality values, original-vs-selected variant metadata,
  metadata-stripped assertion, and thumbnail-refresh assertion. The GUI/effect
  layer may run the actual pixel transform, but it must return selected
  bytes/dimensions/thumbnail through `upload_media`; Tauri then builds
  `UploadMediaRequest.compression` from the current Rust-owned setting.
- `build_upload_media_command` normalizes selected image byte count from
  `bytes.len()` instead of trusting GUI metadata. Phase B compression tests
  should assert the selected variant payload and also check that command Debug
  output redacts filenames, captions, media bytes, and thumbnail bytes.
- The local core media lane prints `upload_staging=ok` and `media_gallery=ok`
  with `send_media=ok`, `media_caption=ok`, `image_compress=ok`, and
  `recv_media=ok`. Those tokens prove the Rust-owned upload-staging/gallery
  contracts only; codec/canvas/native transform behavior and the visible
  drag-drop/paste/gallery/viewer workflow must be covered by browser-headless
  plus Linux virtual-display evidence.
- `TimelinePaneState` includes `staged_uploads` and `media_gallery`. When the
  Rust snapshot adds fields, update the TypeScript snapshot fixtures and
  `apps/desktop/src-tauri/src/dto.rs` serialization-contract tests in the same
  change. GUI Phase B must render these Rust projections and dispatch typed
  commands only; do not keep upload staging/gallery maps in React, synthesize a
  gallery from DOM rows, or parse Matrix media events in the webview.

## Linux GUI QA Container

- Build the committed lane image with
  `docker build -f docker/linux-gui.Dockerfile -t koushi-desktop-linux-gui:basic-ops .`
- The committed image includes `conduit`, `tuwunel`, and `zstd` so the
  `--scenario=local-login` and `--scenario=local-send` lanes can run against
  local homeservers entirely inside the container.
- The Docker recipe pins Rust toolchain `1.96.0` for reproducibility.
- The lane image includes `libnss-wrapper` so the numeric container UID can be
  given a temporary passwd/group entry during DBus-authenticated GUI smoke.
- Run the lane from the repo root with the workspace mounted at `/work`:
  `docker run --rm -it --shm-size=2g -u "$(id -u):$(id -g)" -v "$PWD:/work" -v /tmp/koushi-desktop-cargo-home:/tmp/cargo-home -v /tmp/koushi-desktop-gui-target:/tmp/koushi-desktop-gui-target -v /tmp/koushi-desktop-npm-cache:/tmp/npm-cache -w /work -e HOME=/tmp -e RUSTUP_HOME=/opt/rustup -e CARGO_HOME=/tmp/cargo-home -e CARGO_TARGET_DIR=/tmp/koushi-desktop-gui-target -e NPM_CONFIG_CACHE=/tmp/npm-cache -e PATH=/opt/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin koushi-desktop-linux-gui:basic-ops bash -c 'export RUSTC="$(rustup which rustc)"; export RUSTDOC="$(rustup which rustdoc)"; npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-send --server=conduit --artifact-dir=/work/artifacts/linux-gui-local-send-docker --timeout-ms=180000'`
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
- Local homeserver QA runners resolve `conduit` and `tuwunel` from the child
  process `PATH`; they do not maintain a separate absolute-path probe list.
  Prepend local QA binary directories before running headless/local GUI lanes.
- The canonical durable search-path list lives in
  [docs/qa/headless-basic-operations.md](docs/qa/headless-basic-operations.md#local-homeserver-binary-search-path);
  keep this operational note in sync when changing it.
- Search path list for local homeserver binaries:
  - Host fast lane, preferred:
    `/tmp/koushi-desktop-local-qa-bin`
  - Host fallback/test binaries:
    `/tmp/koushi-desktop-local-qa-bin-test`
  - Docker lane:
    `/usr/local/bin` inside the committed Linux GUI image
  - Windows/manual equivalent:
    `%TEMP%\koushi-desktop-local-qa-bin` or another synthetic, ignored QA bin
    directory prepended to `PATH`
  - Existing user/system `PATH` entries after the QA bin directories
- POSIX host example:
  `export PATH=/tmp/koushi-desktop-local-qa-bin:/tmp/koushi-desktop-local-qa-bin-test:$PATH`
- Quick verification:
  `PATH=/tmp/koushi-desktop-local-qa-bin:$PATH conduit --version` and
  `PATH=/tmp/koushi-desktop-local-qa-bin:$PATH tuwunel --version`.
- Build the debug app once, then reuse it with `--skip-build` (optionally
  `--app-binary=PATH`) so each scenario trial skips the full Tauri rebuild:
  `npm --prefix apps/desktop run tauri build -- --debug --no-bundle`, then
  `PATH=/tmp/koushi-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-create-room --server=conduit --skip-build --artifact-dir=artifacts/linux-gui-local-create-room-fast --timeout-ms=180000`
- Settings/composer-shortcut GUI changes use the same fast loop with
  `--scenario=local-settings`. This opens the real Settings UI, changes the
  Rust-owned composer shortcut and theme settings, verifies the E2EE trust
  settings section renders in the real Tauri WebView, and waits for
  `aria-pressed="true"` / `data-theme="dark"` from the snapshot-driven UI:
  `PATH=/tmp/koushi-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-settings --server=conduit --skip-build --artifact-dir=artifacts/linux-gui-local-settings-fast --timeout-ms=180000`
- Rich formatted timeline rendering uses `--scenario=local-rich-formatting`.
  It sends a sanitized Matrix HTML event through the local homeserver, verifies
  the real Tauri WebView renders the Rust-owned formatted DTO (`strong`,
  blockquote/list/link/code block/copy control), then toggles
  `display.code_block_wrap` through Settings and waits for the code block CSS
  to switch from `pre-wrap` to `pre`:
  `PATH=/tmp/koushi-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-rich-formatting --server=conduit --skip-build --artifact-dir=artifacts/linux-gui-local-rich-formatting-fast --timeout-ms=180000`
- Media GUI iteration has a focused virtual-display lane:
  `--scenario=local-media`. It writes a synthetic fixture file under the
  scenario artifact directory, sets that path on the Composer's hidden file
  input, uses a `DataTransfer` fallback when WebKit leaves `input.files` empty,
  stages the attachment until Send, fills the Rust-owned staging dialog caption,
  waits for `timeline_room=true` and the Rust-owned `TimelineItem.media` row
  plus caption in the real Tauri WebView, clicks Download, opens the
  Rust-owned room media gallery/viewer, and prints `gui_local_media_stage=ok`,
  `gui_local_media=ok`, `gui_local_media_caption=ok`, and
  `gui_local_media_viewer=ok`:
  `PATH=/tmp/koushi-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-media --server=conduit --skip-build --artifact-dir=artifacts/linux-gui-local-media-fast --timeout-ms=180000`
- Image-compression GUI iteration has a focused virtual-display lane:
  `--scenario=local-image-compression`. It sets Compress images to Always from
  the real User Settings panel, returns to the QA Seed Room, attaches an
  ignored synthetic wide PNG, waits for the Rust-owned timeline media row to
  show the compressed `.jpg` filename, `image/jpeg`, and selected dimensions,
  and prints `gui_local_image_compress=ok`:
  `PATH=/tmp/koushi-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-image-compression --server=conduit --skip-build --artifact-dir=artifacts/linux-gui-local-image-compression-fast --timeout-ms=180000`
- Room-tag GUI iteration has a focused virtual-display lane:
  `--scenario=local-room-tags`. It opens the real room row context menu in the
  Linux Tauri WebView, clicks Add/Remove Favourites, and waits for the row to
  move between Rooms and Favourites from Rust-owned `RoomSummary.tags`; it
  prints `gui_local_room_tag_set=ok` and `gui_local_room_tag_removed=ok`:
  `PATH=/tmp/koushi-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-room-tags --server=conduit --skip-build --artifact-dir=artifacts/linux-gui-local-room-tags-fast --timeout-ms=180000`
- Room-management GUI iteration has a focused virtual-display lane:
  `--scenario=local-room-management`. It seeds a helper member, opens the real
  Room info panel, edits the topic, waits for
  `AppState.room_management.settings.topic`, changes the helper role through
  the Rust-owned power-level command, kicks the helper, and waits for the
  room-scoped `settings.members` snapshot to remove the row. It prints only
  `gui_local_room_topic=ok`, `gui_local_room_role=ok`, and
  `gui_local_room_kick=ok`:
  `PATH=/tmp/koushi-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-room-management --server=conduit --skip-build --artifact-dir=artifacts/linux-gui-local-room-management-fast --timeout-ms=180000`
- Explore GUI iteration has a focused virtual-display lane:
  `--scenario=local-explore`. It creates a synthetic public-room fixture with a
  helper account, drives the real Explore search and Join controls, waits for
  Rust-owned directory results and joined room-list state, and prints only
  `gui_local_explore_query=ok` / `gui_local_explore_join=ok`:
  `PATH=/tmp/koushi-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-explore --server=conduit --skip-build --artifact-dir=artifacts/linux-gui-local-explore-fast --timeout-ms=180000`
- Activity GUI iteration has a focused virtual-display lane:
  `--scenario=local-activity`. It opens the real Activity rail entry and
  switches between Unread and Recent tabs through the Tauri command path. It
  prints `gui_local_activity_open=ok`, `gui_local_activity_unread_tab=ok`, and
  `gui_local_activity_recent_tab=ok`:
  `PATH=/tmp/koushi-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-activity --server=conduit --skip-build --artifact-dir=artifacts/linux-gui-local-activity-fast --timeout-ms=180000`
- Message-action/redacted-visibility GUI iteration has a focused
  virtual-display lane: `--scenario=local-message-actions`. It opens the real
  hover-gated message action menu, verifies source/forward behavior, redacts a
  synthetic message, toggles `Hide deleted messages`, and waits for the
  Rust-owned `TimelineItem.is_hidden` projection to remove the redacted row.
  After rebuilding once for frontend/Rust changes, use:
  `PATH=/tmp/koushi-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-message-actions --server=conduit --skip-build --artifact-dir=artifacts/linux-gui-local-message-actions-fast --timeout-ms=180000`
- Composer GUI iteration has a focused virtual-display lane:
  `--scenario=local-composer`. It seeds a synthetic helper member, waits for
  Rust-owned `ProfileState.users` to feed the mention autocomplete, drives the
  real composer mention option, Bold toolbar, and slash input, then waits for
  Rust-owned send state (`send=sent`) plus composer clear. It prints `gui_local_mention=ok`,
  `gui_local_markdown=ok`, and `gui_local_slash=ok`:
  `PATH=/tmp/koushi-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-composer --server=conduit --skip-build --artifact-dir=artifacts/linux-gui-local-composer-fast --timeout-ms=180000`
- Scheduled-send GUI iteration has a focused virtual-display lane:
  `--scenario=local-scheduled-send`. It drives the real Composer `Send later`
  affordance, fills the `datetime-local` control through the shared WebDriver
  setter, waits for Rust-owned scheduled-send snapshots after create/edit/cancel,
  and prints only `gui_local_scheduled_create=ok`,
  `gui_local_scheduled_reschedule=ok`, and
  `gui_local_scheduled_cancel=ok`:
  `PATH=/tmp/koushi-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-scheduled-send --server=conduit --skip-build --artifact-dir=artifacts/linux-gui-local-scheduled-send-fast --timeout-ms=180000`
- Timeline navigation GUI iteration has a focused virtual-display lane:
  `--scenario=local-timeline-navigation`. It drives the real WebView
  first-unread pill, bottom pill, and jump-to-date focused-context path over
  Rust-owned `NavigationUpdated` state. It prints
  `gui_local_timeline_unread_jump=ok`,
  `gui_local_timeline_bottom_jump=ok`, and
  `gui_local_timeline_date_jump=ok`:
  `PATH=/tmp/koushi-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-timeline-navigation --server=conduit --skip-build --artifact-dir=artifacts/linux-gui-local-timeline-navigation-fast --timeout-ms=180000`
- Local alias GUI iteration has a focused virtual-display lane:
  `--scenario=local-alias`. It seeds a synthetic helper sender, opens the real
  hover-gated message sender menu, sets a local alias through the typed
  `set_local_user_alias` path, waits for Rust-projected timeline/member labels,
  clears through the real Room info member list, and waits for both surfaces to
  revert. It prints only `gui_local_alias_set=ok` and
  `gui_local_alias_clear=ok`; do not print Matrix IDs, event IDs, alias values,
  account-data payloads, or raw SDK errors:
  `PATH=/tmp/koushi-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-alias --server=conduit --skip-build --artifact-dir=artifacts/linux-gui-local-alias-fast --timeout-ms=180000`
- CJK visual fitting has a focused virtual-display lane:
  `--scenario=local-cjk`. It creates a long Japanese/CJK local room name, sends
  a long Japanese/CJK message through the real composer, and verifies the Tauri
  WebView CSS contract (`line-break: strict`, `word-break: normal`,
  `hyphens: none`, room ellipsis, message wrapping, no horizontal document
  overflow). It prints `gui_local_cjk=ok`:
  `PATH=/tmp/koushi-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-cjk --server=conduit --skip-build --artifact-dir=artifacts/linux-gui-local-cjk-fast --timeout-ms=180000`
- When you only need a quick window-state sanity check, use the lane's cheap
  QA title helpers such as `--qa-title-ready` and `--qa-title-send-ready`
  before starting a full scenario run.
- Use focused scenarios first. Keep the artifact directories scenario-specific
  so retries do not blur login and send results:

  ```bash
  PATH=/tmp/koushi-desktop-local-qa-bin:$PATH \
    node scripts/desktop-linux-gui-qa.mjs --check-tools

  PATH=/tmp/koushi-desktop-local-qa-bin:$PATH \
    node scripts/desktop-linux-gui-qa.mjs --list

  PATH=/tmp/koushi-desktop-local-qa-bin:$PATH \
    npm --prefix apps/desktop run qa:linux-gui -- \
      --scenario=local-login \
      --server=conduit \
      --artifact-dir=artifacts/linux-gui-local-login-host \
      --timeout-ms=180000

  PATH=/tmp/koushi-desktop-local-qa-bin:$PATH \
    npm --prefix apps/desktop run qa:linux-gui -- \
      --scenario=local-send \
      --server=conduit \
      --artifact-dir=artifacts/linux-gui-local-send-host \
      --timeout-ms=180000
  ```
- Invite/DM GUI iteration has a focused virtual-display lane:
  `--scenario=local-invites-dm`. It seeds a second synthetic local user, accepts
  a real invite through the Invites pane, and starts a DM through the New DM
  dialog. The lane waits for `data-room-kind="dm"` in the real room list, so
  keep `RoomButton`'s data attributes in sync with the Rust-owned sidebar
  snapshot if the room list markup changes. This lane intentionally forces the
  legacy sync backend for deterministic WebDriver smoke; keep using the core
  `invites_dm` QA for SyncService/legacy invite-projection correctness.
- Message type GUI iteration has a focused virtual-display lane:
  `--scenario=local-message-types`. It injects `m.emote`, `m.notice`, and
  formatted spoiler events through the local homeserver helper and verifies the
  real WebView DOM (`data-message-kind`, collapsed spoiler, reveal). After
  changing frontend render code, run one non-`--skip-build` lane before reusing
  `--skip-build`; otherwise the old debug binary can miss new DOM contracts.
- Reuse the existing Cargo, npm, and GUI target caches during the inner loop;
  do not rebuild the Docker image for every trial.
- Run Docker only when you need the committed reproducible lane or want to
  prove the release/CI recipe end to end. It is not the default fast
  iteration path.

## Linux GUI Local Operation Failures

- `--skip-build` reuses an existing debug binary, but the QA window-title
  tokens (`koushi-desktop qa session=...`) are baked into the frontend at build
  time behind `VITE_KOUSHI_QA_TITLE=1`. A binary built without that env
  shows the normal product title (e.g. `Koushi · 1 unread`) instead, so
  the lane's `waitForLocalLoginReady` times out with "local GUI login did not
  reach a ready state. Last title: Koushi · 1 unread". The runner's own
  build sets this env; when pre-building manually for `--skip-build`
  (`npm --prefix apps/desktop run tauri build -- --debug --no-bundle`), also set
  `VITE_KOUSHI_QA_TITLE=1`, or run one lane without `--skip-build` first
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
- The TypeScript snapshot shape is also hand-maintained in
  `apps/desktop/src/domain/types.ts`, `browserFakeApi.ts`,
  `tauriIpcMock.ts`, and `appHarnessMain.tsx`. When adding Rust `AppState`
  fields such as `e2ee_trust` or `invites`, update all mock/default snapshots
  in the same change and run `npm --prefix apps/desktop run typecheck`.
- New `CoreEvent` variants must be wired through the Tauri adapter's
  `serialize_core_event`, TypeScript `coreEvents.ts`, and the checked-in
  `coreEvents.generated.json` contract artifact. The src-tauri
  `core_event_wire_format_matches_checked_in_contract_artifact` test catches
  drift.
- Room-management GUI work must render only `AppState.room_management`.
  Settings snapshots and permission facts are Rust-owned; React should disable
  controls from `settings.permissions` and dispatch typed commands, but it must
  not decide or repair permission, setting, or kick/ban/unban state locally.
  Tauri room-management commands wait for correlated `RoomEvent`s and must not
  call SDK wrappers directly.
- SDK room-setting state events can return before the SDK room cache reflects
  the just-sent state event. The success snapshot must project the submitted
  setting change or wait for a refreshed cache; do not make React patch the
  visible room-management state after a command returns.
- The headless `room_management` scenario uses a disposable management room so
  topic edits and moderation membership changes do not disturb timeline,
  room/space, or reply stages. Permission-guard QA must observe both the
  `OperationFailed(Forbidden)` event and the failed `room_management`
  snapshot; event delivery can lead the connection snapshot by one
  `StateChanged` event.
- Room-management QA output must stay private-data-free: no room IDs, user IDs,
  room names/topics, avatar URLs, moderation reasons, event IDs, or raw SDK
  errors. Success output is limited to `room_settings=ok`,
  `permission_guard=ok`, `moderation=ok`, and cleanup tokens.
- E2EE trust Phase A commands/events are Rust-owned contracts only until the
  AccountActor SDK implementation lands. The fixture/demo backend should return
  typed unavailable/failure actions for trust effects and must not silently
  discard them.
- In production, `CoreCommand::Account` E2EE trust commands must be projected
  through the reducer before `AccountActor` routing. If this is skipped, the
  GUI can only infer pending trust state locally, violating the Rust-owned state
  machine rule.
- `koushi-sdk` is the SDK-facing boundary for E2EE trust operations.
  It maps SDK cross-signing/backup states into private-data-free
  `koushi-state` DTOs and redacts SDK error details in `Debug`. Do not
  let raw SDK trust errors, account keys, verification targets, or backup
  version identifiers leak through normal core events or QA output.
- Matrix identity reset can complete immediately or return an SDK auth
  continuation. Model that as Rust-owned `IdentityResetState`
  (`Idle`, `Resetting`, `AwaitingAuth`, `Failed`), not as React-local state or a
  nullable request id. `AwaitingAuth` exposes only UIAA/OAuth/unknown auth type;
  the SDK handle stays inside `AccountActor` and must be cancelled on logout,
  account switch, and actor shutdown. Auth continuation submission must be a
  `CoreCommand::Account` path that projects `ResetIdentityAuthSubmitted`
  through the reducer before actor routing; the GUI must not own SDK/UIAA/OAuth
  continuation semantics.
- If an E2EE trust `CoreCommand::Account` operation has already projected
  pending reducer state but the actor cannot complete it (session mismatch,
  unavailable local encryption, or an unimplemented SDK path), the actor must
  also send the matching reducer failure action. An `OperationFailed` event
  alone leaves Rust-owned pending state stuck and pushes recovery semantics
  toward the GUI.
- WebDriver `waitForDisplayed`/`click` does NOT reveal hover-gated controls.
  Timeline row actions (`.message-action` inside `.message-actions`) are
  `opacity:0` until `.message:hover`/`:focus-within`, so a direct
  `waitForDisplayed` on the reply button times out ("still not displayed") even
  though the headless Playwright tier passes (its click implicitly hovers).
  Move the pointer first: `await el.waitForExist(); await el.moveTo(); await
  el.waitForDisplayed(); await el.click();`.
- WebDriver native clicks can still be flaky on nested absolute menu items
  inside hover-gated timeline actions. If the menu is visible and exact labels
  are present but native click reports `element not interactable`, use a
  scenario-local helper that finds the visible `button[role="menuitem"]` by
  exact text and dispatches a DOM click. Keep this fallback limited to GUI QA
  plumbing; product code must still use typed Rust commands.
- `send_text` must route through the SDK UI `Timeline::send` path, not a direct
  `room.send_queue().send` call. The latter can settle `SendCompleted` while
  starving the event-driven `TimelineView` of local-echo diffs in the Linux
  WebView lane.
- A reply must target a MESSAGE event, not a state event. The timeline includes
  state events (room create, membership) that carry no body; the SDK's
  `make_reply_event` rejects them (app stderr `make_reply_event failed:
  StateEvent`, surfaced as `send=failed`). `TimelineItemRow` therefore gates the
  reply affordance on `item.body !== null`, so only message rows are replyable.
  A `local-reply` lane must send/target a message and reply to that row, not the
  first event row in a fresh room (whose first events are state events).
- WebDriverIO/WebKit `setValue()` did not populate a `datetime-local` control in
  the `local-timeline-navigation` date-jump lane: the DOM input stayed
  `valueLength=0`, `valid=false`, and the app title stayed `panel=closed
  focused=closed`. For this control, use the lane's `setDatetimeLocalValue`
  helper, which sets the native value property and dispatches `input`/`change`,
  then verify with `timelineDateJumpDiagnostics` before clicking submit. Reuse
  the same helper for scheduled-send `datetime-local` controls. The QA
  title includes `focused=closed|opening|open` so future failures distinguish
  command dispatch/focused-context state from plain DOM text waits.
- Timeline reactions are Rust-owned projection state. React must only dispatch
  typed `SendReaction` / `RedactReaction` commands; do not implement toggle
  semantics in the UI, because `Timeline::toggle_reaction` is only an internal
  Rust delegation detail behind the typed boundary. When `AppState` fields or
  the command surface changes, keep the Tauri DTO, TypeScript domain types, IPC
  mock, browser fake, and serialization-contract tests in sync in the same
  change.
- `local-media` must not use the visible Attach button to open a native file
  dialog. WebDriver should write an ignored synthetic fixture file in the
  scenario artifact directory, set that path on
  `input[type=file][aria-label="Attach file input"]`, fall back to
  `DataTransfer.files` if WebKit does not populate `input.files`, confirm no
  `.message-media` row appears before Send, fill the staged upload caption
  field, then wait for `timeline_room=true` and a Rust-owned media row plus
  caption. It should also open `Open media gallery`, open the uploaded item in
  `Media viewer`, and close the viewer before recording evidence. Do not monkeypatch
  `window.__TAURI_INTERNALS__` from WebDriver; WebKit driver execution contexts
  do not provide a reliable app-world command recorder. If frontend source
  changed, one `local-media` run without `--skip-build` may be needed before
  returning to the fast loop; stale binaries can still show the old immediate
  upload behavior. If the lane fails, inspect the scenario-specific artifact
  run log; the lane uses synthetic filenames/content only and must not write
  real/private media data.
- `local-image-compression` uses a binary-safe `DataTransfer` fallback for the
  synthetic PNG. After changing the image-compression UI or composer upload
  path, run the lane once without `--skip-build` so the QA-title debug binary
  includes the current Media section; a stale binary can fail by never finding
  the Always button. User Settings can unmount the timeline surface in the real
  WebView, so the lane must reselect the QA Seed Room before attaching media.
- `local-room-tags` must use the real context menu and wait for section
  movement from Rust-owned `RoomSummary.tags`. Do not mutate React state,
  monkeypatch Tauri IPC, or treat menu click completion as evidence until the
  row is observed in the expected section.
- `local-room-management` must render member actions from the room-scoped
  `AppState.room_management.settings.members` snapshot, not the global
  profile cache. Member display labels, roles, and power levels are
  Rust-projected facts; React may render a select and dispatch
  `update_room_member_role`, but it must wait for the returned snapshot before
  the visible role changes. Kick/ban success removes the target in the Rust
  reducer; React must not locally filter the member row after command
  completion. The Linux lane stdout must stay
  private-data-free and must not print Matrix room IDs, user IDs, room
  names/topics, avatar URLs, moderation reasons, or raw SDK errors.
- `local-explore` must drive the real Explore pane and wait for
  `AppState.directory.query` results plus the joined room-list snapshot. React
  may keep only the search input draft; it must not synthesize directory
  results, join success, or room-list membership. The Linux lane stdout must
  stay to private-data-free tokens and must not print Matrix aliases, room IDs,
  server names, pagination tokens, or raw SDK errors.
- `local-composer` mention candidates must come from Rust-owned
  `ProfileState.users`, which is projected from SDK room member profiles during
  room-list observation. React may track selected draft mention pills and pass a
  typed `MentionIntent`, but it must not synthesize Matrix `m.mentions`,
  formatted HTML, slash command semantics, or fallback send behavior.
  Timeline mention pills are display-only rendering over Rust-owned timeline
  body text plus `ProfileState.users`; they must not become a React-owned
  source of mention semantics.
- `local-e2ee-key-management` proves #46 Phase B with the real Tauri WebView:
  export a synthetic Matrix/Element-compatible room-key file through the SDK
  path, import that same file, and set up secure backup with recovery-key
  artifact delivery. The lane must print only `gui_room_key_export=ok`,
  `gui_room_key_import=ok`, and `gui_secure_backup_setup=ok`; do not print the
  key-file path, passphrases, recovery key, Matrix IDs, device IDs, room IDs,
  event IDs, message contents, or raw SDK errors. After secure-backup setup, the
  SDK recovery observer can move the session to `needsRecovery`; the right
  panel is then forced to Recovery by Rust-owned session state, so the lane
  accepts either the Settings secure-backup status or QA title
  `panel=recovery session=needsRecovery` as setup evidence.

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
  the product/window title is `Koushi`. GUI automation must check both
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
- First-run GUI smoke should set `KOUSHI_SKIP_SAVED_SESSIONS=1`.
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
  through `KOUSHI_QA_LOGIN_PIPE`, which contains only a FIFO path in the
  environment and keeps the credential payload out of argv, logs, screenshots,
  and committed files.
- Real-login GUI smoke must set `KOUSHI_SKIP_KEYCHAIN_PERSISTENCE=1`.
  `KOUSHI_SKIP_SAVED_SESSIONS=1` only prevents saved-session reads; a
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
  disposable and sets `KOUSHI_SKIP_KEYCHAIN_PERSISTENCE=1`.
  `--qa-profile=<name>` is the opt-in path for persistent restore/sync QA and
  must set `KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR` so unattended runs do
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
