# Implementation Plan Index

Which dated plan governs which area. Read the relevant plan when it governs
the work; use the task-scoped routing in [AGENTS.md](../../AGENTS.md).

Plans are historical once their phase ships — they record the intended sequence
and the deliberate limits of that phase, not the current contract. When a plan
and [state-ownership.md](state-ownership.md) disagree about today's behavior, the
code and the canon win; fix whichever document is wrong.

## Timeline viewport replacement proposal

[Timeline viewport redesign](../superpowers/specs/2026-09-05-timeline-viewport-redesign.md)
defines the proposed replacement for #837/#844: one list engine, explicit
renderer intents, comparative native-motion verification, and large-room
acceptance including Core/media costs. Production behavior is unchanged;
library selection and the normative algorithm amendment require the documented
feasibility gate. Historical plans below are not the new implementation brief.

## Runtime and roadmap

[#1628 MCP message actions](../plans/2026-10-27-issue1628-mcp-message-actions.md)
gives an assistant `get_message` plus react, edit, delete, and thread reply as
direct session-owned operations behind a new `message_actions` grant
(`connections.json` schema 3), with no timeline actor and no GUI state change.

[One-click MCP registration](../plans/2026-10-19-mcp-one-click-register.md)
lets the AI assistants page run Claude Code's, Codex's, or pi's own
registration CLI (resolved with the login `PATH`, bounded and killable) and
build a Claude Desktop MCPB extension; other clients keep copy and paste.

[#1536 bundled MCP launcher and in-app approval](../plans/2026-10-19-issue1536-mcp-approval.md)
replaces hand-edited grants with a consent broker in `koushi-mcp-host`: a
missing permission becomes an approval the user answers in Koushi, per
assistant, account, and permission, recorded in `mcp/connections.json` schema 2.
Adds the `mcp.enabled` switch, the App Settings AI assistants page with
copyable registration text, and the launcher in every release package.

[#1524 MCP/GUI isolation](../plans/2026-10-18-mcp-ui-isolation.md)
makes every MCP tool run as an in-process Core call that never reduces app
actions or emits WebView events: independent per-request searches with the
caller's room filter, attachment retrieval without a timeline actor, and
display-independent sends. Progress, errors and cancellation stay with the
calling MCP client.

[#1520 local MCP bridge](../plans/2026-10-18-issue1520-mcp-bridge.md)
gives Claude Code and Codex encrypted search, sending with attachments, and
polled new-message delivery through a stdio launcher (`crates/koushi-mcp`) over
an authenticated local socket hosted by the Tauri adapter. The bridge attaches
a `CoreConnection` per requested account, never opens a crypto store, keeps
Matrix credentials inside Koushi, and replaces push with a user-configured,
account-bound event cursor.

[#1482 timeline mutation execution](../plans/2026-10-22-issue1482-timeline-mutations.md)
moves edit/redact/reaction operations to a bounded session-owned execution owner,
retains SDK rollback after client expiry, and separates pending insertion from
actor publication. The extended inventory retains recovery/forward HTTP work.

[#1481 room-scoped typing lane](../plans/2026-10-16-issue1481-typing-lane.md)
moves the outbound `Room::typing_notice` HTTP call off the `TimelineActor`
mailbox loop into a session-owned, room-scoped bounded FIFO lane so pending-send
publication and diff batches no longer wait behind an unrelated typing request;
the plan also records the handler audit that leaves the SDK-enqueue and
owner-polled-worker handlers in place.

[#1427 device-local notification delivery](../plans/2026-10-14-issue1427-local-notification-email.md)
makes the app deliver its own notifications and the default email path, because the
server-owned `kind: email` pusher cannot work on a homeserver such as matrix.org: an
app-scoped sink list (a mail provider such as Resend or a QQ Mail SMTP preset, later a
WeChat relay) with every sink secret in the credential store, triggers taken from the
persistent unread state after a configurable dwell, an interval-coalesced in-memory
queue with no durable outbox, and a default-off plaintext prefix behind a
provider-agnostic consent. The server pusher surface keeps its own meaning.

[#1460 notification routes](../plans/2026-10-16-issue1460-notification-routes.md)
supersedes the single device channel: every destination (Resend, SMTP, WeChat via
ServerChan Turbo) owns its targets, content mode, consent, and timing lane, with
no master switch; the settings page stacks OS, this-device, and homeserver
sections, and OS previews are a verified, default-off account setting.

[#1468 notification settings tabs](../plans/2026-10-16-issue1468-notification-settings-tabs.md)
replaces the stacked page with three tabs and a destination list. Add and edit open a
configuration screen whose Save is one atomic `SaveNotificationSink` command: the
non-secret sink, the secret, and a core-minted consent. A failed secret store rolls
back an added destination.

[#1396 avatar thumbnails](../plans/2026-10-13-issue1396-avatar-thumbnails.md)
requests a homeserver thumbnail sized from the 40 px avatar slot, resizes the
fallback locally to the same long edge, keys the cache by thumbnail identity, and
splits the avatar and link-preview budgets (8 MiB vs 32 MiB). It retired a
released source's bytes; [#1432](../plans/2026-10-14-issue1432-avatar-source-retirement.md)
removed the scoped-demand half of that retirement and records that a source
the renderer can still show is never left announced as Ready. Records the open
decisions and the measured output
size.

[#1394 resident timeline-set diagnostics](../plans/2026-10-13-issue1394-resident-timeline-diagnostics.md)lands the export-time counters the memory measurement record asked for before any
retention bound: resident Room/actor counts and a canonical-depth proxy, as
`core.timeline_residency` rows next to `core.search_retention`. Observation only;
the unbounded timeline-actor map and the depth axis stay as recorded.

[#1392 topology-first gap inspection](../superpowers/plans/2026-10-13-issue1392-gap-inspection-topology.md)
describes the SDK-fork store path (`EventCacheStore::load_chunk_topology`) that
removes full-history decryption from healthy room gap inspection, the koushi
counter records, and the benchmark evidence obligations; two PRs split at the
fork/application repository boundary.

[#1363 / #1366 sidebar section row bound](../plans/2026-10-12-issue1363-1366-sidebar-section-bound.md)
bounds each visible section to five rows with a Rust-projected expand control,
stores the per-scope `expanded` preference next to `collapsed`/`sort`, reveals a
collapsed section on conversation entry for the session, and records the
deliberate divergence from Element Web's non-truncating room list.

[#1364 persisted room activity](../plans/2026-10-12-issue1364-persisted-room-activity.md)
records why the Recent sidebar order collapsed into name order, the
`recency_stamp` fallback for the activity-less group, and the persisted per-room
conversation-activity store that the #542 Recent-first default depends on.

[#542 Recent-first default](../plans/2026-10-12-issue542-recent-first-default.md)
makes recent conversation order the default for every scope and section, keeps
`Activity` deserializable for settings an older build persisted, removes the
attention-first selector option, and corrects the canon's attention ranks to the
three the reducer implements.

[#1150 index-first search and bounded memory](../plans/2026-10-06-issue1150-search-index-first.md)
tracks M2: making the persistent ngram index the only candidate source and
removing full-history RAM body/edit retention, with the remaining store shrink
and bounded refill. M3 (durable crawl commitments) and M4 (startup warm set)
were dropped from the PR.

[#1150 resident retention](../plans/2026-10-09-issue1150-resident-retention.md)
records the M2 follow-up: it lands the fork PR #19 revision on the SDK fork's
`main` and the matching gitlink move, and it records why the retention changes
(bounded Room timeline-actor set, event-ring sizing) are **not** implemented
-- two independent reviews rejected the direct design with verified
read-intent-loss, quiescence, teardown-settlement and scheduler hazards. It also
records the open decisions (policy owner, read intent, measurement first) and
what was dropped (the SDK search-reader LRU, a second React retention limit, the
invented 64 MiB and p95/p99 gates, and the conditional SDK gap step).

[#1230 cold Room timeline retirement](../plans/2026-10-09-issue1230-cold-room-retirement.md)
and its successor [read-intent ownership](../plans/2026-10-09-issue1230-read-intent-ownership.md)
are an **archive, not implementation briefs**. Four designs were reviewed and
none reached an implementable state before the maintainer closed the work as not
planned: the retirement-policy designs failed because read work exists
independently of `local_read_correlations`, and the ownership-move revision left
eight specification items open, including an engineering-rules cancel-and-await
conflict. The residual retention is instead tracked as a "Known deviation" note in
engineering rules "Async and Runtime" 2. The first record also corrects a factual
error: Room retirement already has a quiescence fence inside
`clear_thread_root_projections_for_room`.

[#1344 bounded search/history residency](../plans/2026-10-12-issue1344-bounded-search-retention.md)
is the design record for the maintainer's 2026-10-12 direction: durable crawl
progress, bounded history traversal, and operation-scoped search readers. Its
revision 1 was rejected by an independent pre-implementation review; revision 2
dispositions every verified finding, puts the non-hydrating persisted-store
cursor first, and leaves one scope decision open (per-account vs cross-account
byte allowances).

- Rooms / DMs collapsible sidebar sections (design and implementation record):
  [2026-09-19-sidebar-sections-design.md](../superpowers/specs/2026-09-19-sidebar-sections-design.md)
  — its Low priority removal is superseded by #955 below.
- Low priority section restoration and Rooms / DMs unread badges (#955):
  [2026-09-20-issue955-low-priority-and-section-unread.md](../superpowers/plans/2026-09-20-issue955-low-priority-and-section-unread.md)
- macOS auto-update with portable desktop state (#878):
  [2026-09-12-issue878-macos-auto-update.md](../superpowers/plans/2026-09-12-issue878-macos-auto-update.md)
- Scoped receipt-reader vertical (#839/#840/#846):
  [2026-09-06-issue839-scoped-readers.md](../superpowers/plans/2026-09-06-issue839-scoped-readers.md)
- Shared SDK receipt snapshots (#839/#840):
  [2026-09-06-issue839-sdk-receipt-sharing.md](../superpowers/plans/2026-09-06-issue839-sdk-receipt-sharing.md)
- Narrow settled outcome payloads (#840):
  [2026-09-06-issue840-outcome-payloads.md](../superpowers/plans/2026-09-06-issue840-outcome-payloads.md)
- Scalar state-generation reads (#840):
  [2026-09-06-issue840-scalar-generations.md](../superpowers/plans/2026-09-06-issue840-scalar-generations.md)
- Profile display-label mutation identities (#840):
  [2026-09-06-issue840-profile-change-identities.md](../superpowers/plans/2026-09-06-issue840-profile-change-identities.md)
- Bounded AppActor command turns (#840):
  [2026-09-06-issue840-command-turns.md](../superpowers/plans/2026-09-06-issue840-command-turns.md)
- Headless core runtime:
  [2026-06-12-headless-core-runtime-implementation.md](../superpowers/plans/2026-06-12-headless-core-runtime-implementation.md)
- Phase 10+ product surface and release roadmap:
  [2026-06-13-roadmap-phases-10-18.md](../superpowers/plans/2026-06-13-roadmap-phases-10-18.md)
- Local GUI room/space/reply operations:
  [2026-06-13-local-gui-basic-operations.md](../superpowers/plans/2026-06-13-local-gui-basic-operations.md)

## Umbrella #12 — Core Batch A / GUI Batch B

Batch Rust-owned Phase A contracts first, then serialize the shared GUI surface,
then run the #9/#31 integration gate.

- Design/split:
  [2026-06-15-remaining-core-phase-a-batch-design.md](../superpowers/specs/2026-06-15-remaining-core-phase-a-batch-design.md)
- Implementation:
  [2026-06-15-remaining-core-phase-a-batch-implementation.md](../superpowers/plans/2026-06-15-remaining-core-phase-a-batch-implementation.md)

Before starting each new task in that batch, refresh open GitHub issues and apply
the plan's issue reconciliation addendum. New GUI-only presentation items such as
space tooltips do not bypass the Rust-owned Phase A rule for product behavior.

## Feature areas

Phase A is Rust/headless work and comes before Phase B GUI wiring. A `plan`
link means the same plan covers both phases; `Phase A` or `Phase B` alone
means the index recorded only that phase. Rows marked superseded are kept
only because older issues link to them; they do not govern current work.

| Area | Plans |
| --- | --- |
| Media / file timeline | [Phase A](../superpowers/plans/2026-06-15-media-phase-a.md) |
| Right-panel/composer containment (#1119, #1121 Phase B) | [Phase B](../superpowers/plans/2026-10-05-issue1119-panel-containment.md) |
| History export archive: room and Space folders with HTML, attachments, resume | [Phase A](../superpowers/plans/2026-09-25-history-export-archive.md), [spec](../superpowers/specs/2026-09-25-history-export-archive-design.md) |
| Room-history export, Element-compatible JSON (#59) | [Phase A](../superpowers/plans/2026-09-23-issue59-room-history-export-phase-a.md), [Phase B](../superpowers/plans/2026-09-23-issue59-room-history-export-phase-b.md) |
| Media preparation/cache retention (#547) | [plan](../superpowers/plans/2026-08-18-issue547-memory-bounds.md) |
| Muted-room native Dock attention (#543) | [plan](../superpowers/plans/2026-08-18-issue543-muted-dock-badge.md) |
| Rust lifecycle ownership / leak cleanup (#550) | [plan](../superpowers/plans/2026-08-18-issue550-rust-lifecycle-ownership.md) |
| Logical window-state restore (#544) | [Phase B](../superpowers/plans/2026-08-22-issue544-logical-window-state.md) |
| Rust-owned live viewport synchronization (#666) | [plan](../superpowers/plans/2026-08-22-issue666-rust-viewport-synchronization.md) |
| Invite-workflow admission and settlement guards (#658) | [Phase A](../superpowers/plans/2026-08-22-issue658-invite-workflow-admission.md) |
| Composer-load session fence evidence (#645) | [Phase A](../superpowers/plans/2026-08-22-issue645-composer-load-session-fence.md) |
| Browser harness resource lifecycle (#657) | [Phase B](../superpowers/plans/2026-08-22-issue657-harness-resource-lifecycle.md) |
| Bounded KaTeX math rendering (#668) | [Phase B](../superpowers/plans/2026-08-22-issue668-bounded-math-rendering.md) |
| Transient settlements and trust-loss resets (#660) | [Phase A](../superpowers/plans/2026-08-23-issue660-transient-settlement-trust-reset.md) |
| Room-list session-fence acceptance (#659) | [Phase A](../superpowers/plans/2026-08-24-issue659-room-list-session-fence-acceptance.md) |
| Authentication invalidation diagnostics and UI (#608) | [Phase A](../superpowers/plans/2026-08-24-issue608-authentication-invalidation-diagnostics.md) |
| Rust-owned live thread-summary authority (#678) | [plan](../superpowers/plans/2026-08-25-issue678-rust-thread-summary-authority.md) |
| Thread reply quote hydration and panel containment (#1121, #1120, #1119) | [plan](../superpowers/plans/2026-10-05-issue1121-thread-reply-quote-and-panel.md) |
| Historical sender-profile hydration (#688) | [Phase A](../superpowers/plans/2026-08-25-issue688-historical-sender-profiles.md) |
| Secure Backup startup convergence | [plan](../superpowers/plans/2026-08-25-secure-backup-startup-convergence.md) |
| User Settings session/account convergence | [plan](../superpowers/plans/2026-08-25-user-settings-session-convergence.md) |
| Authoritative current-device verification (#694 Priority 1) | [plan](../superpowers/plans/2026-08-25-issue694-authoritative-verification.md) |
| Active-session account management (#694 Priority 2) | [plan](../superpowers/plans/2026-08-26-issue694-active-session-account-management.md) |
| Room-latest redaction/edit convergence (#570 Task C) | [plan](../superpowers/plans/2026-08-24-issue570-room-latest-convergence.md) |
| Local-viewed read-state convergence (#559) | [plan](../superpowers/plans/2026-08-24-issue559-local-viewed-read-state-convergence.md) |
| Deterministic settlement (#738) | [plan](../superpowers/plans/2026-08-28-issue-738-deterministic-settlement.md) |
| Canon and low-risk architecture cleanup (#750) | [Phase A](../superpowers/plans/2026-08-29-issue750-architecture-cleanup.md) |
| Rust source-contract and test-module cleanup (#753) | [Phase A](../superpowers/plans/2026-08-30-issue753-rust-test-structure.md) |
| DM Space-membership readiness (#780) | [Phase A](../superpowers/plans/2026-08-31-issue780-dm-space-membership-readiness.md) |
| Thin Tauri adapter and Core-owned settlement (#755) | [plan](../superpowers/plans/2026-08-30-issue755-thin-tauri-adapter.md) |
| Ordered state transport and renderer-independent settlement (#759) | [plan](../superpowers/plans/2026-09-01-issue759-ordered-state-transport.md) |
| Rust-owned frontend preferences and TypeScript semantic deletion (#761) | [plan](../superpowers/plans/2026-09-04-issue761-rust-owned-preferences.md) |
| Rust-owned Activity event navigation (#836) | [Phase A](../superpowers/plans/2026-09-04-issue836-activity-event-navigation.md) |
| Deterministic README application screenshot (#835) | [Phase B](../superpowers/plans/2026-09-05-issue835-readme-screenshot.md) |
| Desktop polish batch (#806, #826, #827, #828, #831, #832, #833) | [plan](../superpowers/plans/2026-09-04-issues826-827-828-831-832-desktop-polish.md) |
| Frontend-neutral protocol and QA isolation (#763) | [Phase A](../superpowers/plans/2026-09-05-issue763-frontend-neutral-protocol-qa.md) |
| Leaf crate boundaries and Core edge cleanup (#765) | [Phase A](../superpowers/plans/2026-09-05-issue765-leaf-boundaries.md) |
| Activity/edit/redaction convergence (#570 umbrella) | [plan](../superpowers/plans/2026-08-23-issue570-redaction-edit-convergence.md) |
| SDK thread relation aggregate (#570 Task A) | [Phase A](../superpowers/plans/2026-08-23-issue570-sdk-thread-aggregate-spike.md) |
| Space member role management (#582) | [plan](../superpowers/plans/2026-08-23-issue582-space-member-role-management.md) |
| Core Activity/unread/thread convergence (#570 Task B) | [Phase A](../superpowers/plans/2026-08-24-issue570-core-activity-thread-convergence.md) |
| Tauri core-event forwarder lifecycle (#656) | [plan](../superpowers/plans/2026-08-22-issue656-tauri-forwarder-lifecycle.md) |
| Linux GUI new-identity bootstrap QA (#586) | [plan](../superpowers/plans/2026-08-20-issue586-linux-gui-new-identity-bootstrap.md) |
| Live signals (receipts, markers, typing, presence) | [Phase A](../superpowers/plans/2026-06-15-live-signals-phase-a.md), [Phase B](../superpowers/plans/2026-06-15-live-signals-phase-b-gui.md) |
| E2EE trust state machine | [Phase A](../superpowers/plans/2026-06-14-e2ee-trust-phase-a.md) |
| Rust-owned settings | [Phase A](../superpowers/plans/2026-06-14-rust-owned-settings-phase-a.md) |
| i18n substrate | [Phase A](../superpowers/plans/2026-06-14-i18n-substrate-phase-a.md), [Phase B](../superpowers/plans/2026-06-14-i18n-substrate-phase-b.md) |
| Cross-platform font/emoji substrate | [Phase A](../superpowers/plans/2026-06-15-font-emoji-phase-a.md), [Phase B](../superpowers/plans/2026-06-15-font-emoji-phase-b-gui.md) |
| Compact message density (#609) | [Phase B](../superpowers/plans/2026-08-22-issue609-compact-message-density.md) |
| Timeline navigation aids (#41) | [Phase A](../superpowers/plans/2026-06-16-timeline-navigation-phase-a.md) |
| Unread navigation and thread notifications (#569) | [Phase B](../superpowers/plans/2026-08-22-issue569-unread-navigation-thread-notifications.md) |
| Account work scheduler | [Phase A](../superpowers/plans/2026-07-25-account-work-scheduler-phase-a.md) |
| Startup latency observability (#123) | [Phase A](../superpowers/plans/2026-06-23-startup-latency-observability-phase-a.md) |
| Element X-compatible login/store lifecycle (#699) | [Phase A](../superpowers/plans/2026-08-26-issue699-element-x-login-store-lifecycle.md) |
| Initial index-0 key-share diagnostics (#509) | [Phase A](../superpowers/plans/2026-08-13-index0-share-diagnostics.md) |
| Bounded index-0 duplicate share (#510) — superseded by stock Element X pre-share, #795 | [Phase A](../superpowers/plans/2026-08-13-index0-reshare.md) |
| Initial Megolm Olm-claim repair (#523) — superseded by stock Element X pre-share, #795 | [Phase A](../superpowers/plans/2026-08-14-initial-megolm-olm-repair.md) |
| Element X Megolm send parity (runtime-disable #510/#523) | [Phase A](../superpowers/plans/2026-08-15-element-x-megolm-send-parity.md) |
| Room-subscription ownership (#518) | [Phase A](../superpowers/plans/2026-08-14-room-subscription-ownership.md) |
| Session-resident room subscriptions (#532) | [Phase A](../superpowers/plans/2026-08-15-room-subscription-residency.md) |
| Room-key rotation correlation diagnostics | [Phase A](../superpowers/plans/2026-08-14-room-key-rotation-correlation-diagnostics.md) |
| Eviction-resistant Megolm rotation attribution (#591) | [plan](../superpowers/plans/2026-08-21-issue591-rotation-ledger.md) |
| Persisted Megolm rotation attribution (#794) | [Phase A](../superpowers/plans/2026-09-05-issue794-persisted-rotation-attribution.md) |
| Element X Megolm send parity (#795) | [Phase A](../superpowers/plans/2026-09-05-issue795-element-x-megolm-send-parity.md) |
| Stock forced rotation debug control / JS-error attribution (#797) | [plan](../superpowers/plans/2026-09-06-force-rotation-issue797-js-errors.md) |
| New-session Megolm readiness — phase 1 (#577) — superseded; readiness fences are prohibited by overview "Initial outbound Megolm delivery" | [Phase A](../superpowers/plans/2026-08-21-issue577-megolm-readiness.md) |
| Same-user secondary-device QA credential isolation (#577 follow-up) | [Phase A](../superpowers/plans/2026-08-21-issue577-secondary-device-qa-credentials.md) |
| Formatted-body newline preservation (#522) | [Phase A](../superpowers/plans/2026-08-14-formatted-body-newlines.md) |
| Nested Markdown bullet lists (#648) | [Phase A](../superpowers/plans/2026-08-22-issue648-nested-markdown-lists.md) |
| Unified renderer viewport stabilization (#837) | [Phase B](../superpowers/plans/2026-09-05-issue837-viewport-transaction.md) |
| Active prepend anchor preservation (#520) | [Phase A](../superpowers/plans/2026-08-14-active-prepend-anchor.md) |
| Feature-seam decomposition wave (#551, 51 indexed plans) | [first plan](../superpowers/plans/2026-08-18-issue551-feature-seam-decomposition.md); children are `docs/superpowers/plans/2026-08-*-issue551-*.md` |
| Frontend semantic-ownership migration (#552 phases 1-7, #708, 22 plans) | [inventory](../superpowers/plans/2026-08-23-issue552-frontend-ownership-inventory.md), [remaining phases](../superpowers/plans/2026-08-27-issue552-remaining-ownership-phases.md), [#708 thread-root phase 1](../superpowers/plans/2026-08-27-issue708-thread-root-projection-ownership.md); children are `docs/superpowers/plans/2026-08-*-issue552-*.md` |
| Browser-fake cleanup (#634, #641, #649, #650, #651, 9 plans) | [first plan](../superpowers/plans/2026-08-22-issue634-browser-fake-link-preview-isolation.md); children are `docs/superpowers/plans/2026-08-22-issue6*-browser-fake-*.md` |
