# Remaining Core Phase A Batch Design

Date: 2026-06-15
Status: approved design for reorganizing the remaining issue #12 work before
implementation planning.

## Goal

Reorganize the remaining umbrella issue #12 work so the Rust-owned Phase A
state-machine and headless contracts can be developed in larger, parallelizable
batches before the shared GUI surface is touched.

This design changes execution order and ownership only. It does not close any
feature issue by itself, and it does not permit React to own product semantics.
Each child issue remains the unit of acceptance and closure.

## Decision

The remaining roadmap is split into three execution layers:

1. **Core Batch A** - Rust state, commands, actors, DTOs, reducers, Tauri
   transport contracts, headless tokens, and canon updates for multiple issues
   are implemented before GUI work.
2. **GUI Batch B** - React/Tauri UI is serialized through the shared surfaces:
   `App.tsx`, `TimelineView.tsx`, room-list/sidebar components, `styles.css`,
   i18n labels, and browser-headless GUI-operation tests.
3. **QA Batch Z** - issue #9 and #31 run last as the integrated gate and
   product walkthrough after feature-specific Core and GUI evidence exists.

Feature issues stay feature-scoped. Implementation plans may group Phase A
work across issues, but issue comments and close criteria must still record
which feature slice landed, which checks prove it, and which Phase B work
remains.

## Alternatives Considered

### Keep One Issue At A Time

This minimizes local reasoning but serializes too much Rust work behind the
GUI timeline. It also encourages implementers to jump into React early because
each issue asks for an end-to-end user-visible slice.

### Split Every Issue Into A Separate Agent

This looks parallel, but several issues touch the same hot files:
`state.rs`, `action.rs`, `reducer.rs`, `command.rs`, `runtime.rs`, Tauri DTOs,
TypeScript wire types, and contract artifacts. Uncoordinated agents would
produce conflicts and drift in the state-machine canon.

### Approved Approach: Batched Core With Main-Agent Integration

Subagents own narrow Rust/core modules or investigations in parallel. The main
agent owns state-machine canon, shared DTO/action/command enums, generated
contract artifacts, review, issue status, and final gate execution. This
preserves parallel speed while keeping the central product state coherent.

## Batch Topology

### Core Batch A0 - Decisions And Contract Allocation

Purpose: settle decisions that affect several later slices and reserve the
shared state/command/event shape before subagents edit code.

Included issues:

- **#30**: decide backup restore semantics as recovery secret import plus
  joined-room key hydration for MVP. Do not add a vendored SDK accessor for
  exhaustive backup-wide restore unless a later product decision explicitly
  requires it.
- **#32 Stream 1**: define the Japanese catalog completeness rule.
- **#7**: define local encryption / credential-store health states.
- **#10**: define notification attention, badge, sound, tray, and activation
  state as Rust-owned candidate/capability data.

Main-agent responsibilities:

- Amend canon before implementation when a decision changes architecture.
- Allocate shared enum/DTO names and request-correlation conventions.
- Keep private-data-free failure kinds and Debug redaction rules explicit.

### Core Batch A1 - Timeline And Composer Core

Purpose: finish timeline/composer product semantics that otherwise tempt the
GUI to synthesize state.

Included issues:

- **#19 Phase A1**: use
  `docs/superpowers/specs/2026-06-15-message-interactions-phase-a1-design.md`
  as the binding spec for reply quotes and pinned events.
- **#19 follow-up Phase A slices**: design and later implement permalink,
  copy/view-source safe DTOs, forward-message command, and link-preview
  projection separately from A1. Link preview is the most sensitive slice
  because it needs cache, fetch, and privacy policy.
- **#18 Phase A**: intentional mentions, markdown/rich-text send semantics,
  slash-command parsing, autocomplete data contracts, and shared composer
  key-event behavior.
- **#32 Stream 3**: make `is_composing` part of the shared composer key-event
  contract across main, thread, edit, and autocomplete contexts.

Rust-owned decisions:

- Mention payloads are derived by Rust from structured composer intent and
  member/room suggestions; React may render pills and keep unsent draft text,
  but it does not decide final `m.mentions`.
- Markdown/rich-text conversion and sanitization are Rust-owned send-path
  semantics. React may offer toolbar controls but dispatches typed intent.
- Slash commands parse in Rust before Matrix commands are emitted. Unknown or
  unsupported commands settle as structured local failures, not UI heuristics.
- Reply quote, pinned-event, permalink, source, forward, and preview data cross
  the GUI boundary only as app-owned DTOs.

Likely headless tokens:

- `reply_quote=ok pin_event=ok pinned_state=ok unpin_event=ok`
- `mention_send=ok markdown_send=ok slash_command=ok ime_guard=ok`

### Core Batch A2 - Rooms, Directory, Management, Activity

Purpose: keep room membership, directory, moderation, and cross-room activity
semantics out of the sidebar and right-panel React code.

Included issues:

- **#20 Phase A**: public room directory query state, pagination, result DTOs,
  join-by-alias/server-name rules, and error taxonomy.
- **#21 Phase A**: room settings snapshot, power-level/permission state,
  guarded room-setting mutations, and moderation commands.
- **#23 Phase A**: Rust-owned Activity view with Recent and Unread streams,
  cross-room ordering, per-room unread bounds, pagination, focused-context jump,
  and mark-read commands.
- **#22 Phase B handoff**: room tag semantics are already Rust-owned; GUI
  section movement must consume `RoomSummary.tags` only.

Rust-owned decisions:

- Directory results do not become room membership until a Rust join result
  updates room state.
- Moderation button visibility may be rendered by React, but permission facts
  and command acceptance are Rust-owned.
- Activity Recent and Unread share cross-room infrastructure but keep separate
  bounds. Viewing Unread does not mark read; jump/mark-read commands do.
- Muted and low-priority exclusions come from Rust room state, not UI filters.

Likely headless tokens:

- `directory_query=ok directory_join=ok`
- `room_settings=ok moderation=ok permission_guard=ok`
- `activity_recent=ok activity_unread=ok activity_markread=ok`

### Core Batch A3 - Platform, Security, Localization, Notifications

Purpose: settle platform-capability and localization semantics before Settings,
security, notifications, or CJK GUI work.

Included issues:

- **#7 Phase A**: credential-store availability/status, fail-closed states,
  reset-local-data allowance, and OS credential capability profile.
- **#10 Phase A**: notification settings, room/space/thread attention summary,
  native attention candidate, badge count, sound/tray capability state, and
  dedupe/suppression rules.
- **#30**: document joined-room restore semantics and prevent "full restore"
  wording in product state or QA.
- **#32 Streams 1 and 2**: Japanese catalog completeness, CJK normalization,
  CJK-aware Rust-owned sort keys for room/person ordering, and search/display
  matching policy.

Rust-owned decisions:

- Credential-store health is a serializable Rust state with coarse
  private-data-free failure kinds. Raw OS/keyring errors never cross the
  boundary.
- Notification candidates contain only redacted/minimized fields allowed by
  `REPOSITORY_RULES.md` and `docs/policies/engineering-rules.md`.
- Platform differences are represented by capability/profile DTOs, not by
  component-level OS branches.
- Locale fallback, Japanese completeness, CJK normalization, and collation are
  resolved before GUI rendering.

Likely headless tokens:

- `credential_health=ok fail_closed=ok`
- `notification_candidate=ok badge_state=ok suppress_focus=ok clear_badge=ok`
- `ja_catalog=ok cjk_normalize=ok cjk_collation=ok`

## Parallel Agent Model

Subagents may run in separate worktrees or isolated branches. They must not
edit the same hot file without main-agent coordination.

Recommended ownership:

- **Agent 1 - timeline/composer core**: #19 A1 and #18/#32 Stream 3 Phase A
  investigations or module-local patches.
- **Agent 2 - room/discovery/activity core**: #20/#21/#23 Phase A
  investigations or module-local patches.
- **Agent 3 - platform/security/i18n/notification core**: #7/#10/#30/#32
  Streams 1-2 investigations or module-local patches.
- **Main agent**: shared enums, reducers, command/event integration, Tauri DTOs,
  generated contract artifacts, canon, verification, issue comments, and close
  decisions.

When a subagent needs to touch shared files, it reports the desired contract
shape instead of directly merging changes. The main agent integrates shared
state updates in a controlled pass.

## Shared Hot Files

These files are serialization points:

- `crates/koushi-state/src/state.rs`
- `crates/koushi-state/src/action.rs`
- `crates/koushi-state/src/reducer.rs`
- `crates/koushi-core/src/command.rs`
- `crates/koushi-core/src/event.rs`
- `crates/koushi-core/src/runtime.rs`
- `apps/desktop/src-tauri/src/dto.rs`
- `apps/desktop/src/domain/coreEvents.ts`
- `apps/desktop/src/domain/coreEvents.generated.json`
- `apps/desktop/src/i18n/messages.ts`
- `apps/desktop/e2e/basic-operations.spec.ts`

GUI files are also serialization points and remain in GUI Batch B:

- `apps/desktop/src/App.tsx`
- `apps/desktop/src/components/TimelineView.tsx`
- room-list/sidebar/right-panel components
- `apps/desktop/src/styles.css`

## Issue Restructuring

The GitHub issues remain feature-scoped, but the umbrella execution order is
updated:

1. Core Batch A0/A1/A2/A3 design and Phase A implementation.
2. Serialized GUI Batch B feature slices.
3. QA Batch Z integration and product walkthrough.

Child issues should receive comments when their Phase A portion lands. They are
closed only after their Phase B GUI-operation checks and required Linux
virtual-display evidence are present, unless the issue is explicitly a
decision-only item such as #30.

### 2026-06-15 Reconciliation Addendum

The issue inventory changed after this batch design was approved. The execution
model remains valid, but the umbrella scope must include the later roadmap
issues and classify them by the same Phase A / Phase B ownership rule:

- **#64 read-receipt reader avatars** is a normal parity gap under the already
  closed #16 area. It needs a Rust-owned receipt reader projection first
  (reader display label, avatar URL, timestamp, ordering, overflow), then a
  serialized TimelineView GUI slice. It does not block #23 Activity core.
- **#65 space icon tooltip** is GUI-only because tooltip visibility, delay, and
  positioning are ephemeral presentation state. The displayed name still comes
  from Rust-owned `space.display_name`, so no new Core Batch A contract is
  required.
- **#63 local/personal user aliases** is a distinctive feature tracked by #62.
  It needs its own Rust-owned account-data-backed alias map and name-resolution
  projection before any GUI work. Because it affects timeline senders, member
  lists, DM titles, receipts, reply quotes, mentions, and notifications, it
  should run after the base projection contracts are stable and before those
  GUI surfaces claim alias completeness.
- **#62 distinctive features** remains a living index, not a directly closable
  implementation unit. Promoted child issues such as #56, #58, #60, and #63
  are the acceptance units.
- **#7 credential-store health** now includes a verification-tier decision:
  Tier 1 is trait-backed fake/in-memory logic and status tests on any OS; Tier
  2 is an env-gated macOS temporary-Keychain integration lane; Tier 3 is
  attended-only consent/Touch ID/signed-build behavior.

## Required Follow-Up Specs

This batch design is not a substitute for every feature's implementation plan.
Before writing code, implementation planning should either reference an existing
approved spec or create one of these narrower specs:

- #18: composer mentions, markdown, and slash-command design.
- #19 follow-ups after A1: message actions, permalink/source/forward, and link
  preview design.
- #20: public directory and Explore design.
- #21: room settings and moderation design.
- #23: Activity view design, promoting the detailed issue-body agreement into
  repo docs.
- #7: credential-store health design.
- #10: notification/native attention design extension.
- #32: Japanese/CJK design covering catalog, normalization, collation, and IME
  resolver behavior.
- #64: read-receipt reader-avatar projection and Tooltip reuse design.
- #63: local/personal alias account-data and name-resolution design.

The implementation plan may group these specs into one Core Batch A plan when
the shared contract allocation is clear.

## Verification Strategy

Core Batch A verification is Rust/headless first:

- reducer tests for every new state machine, including stale request IDs,
  duplicate completions, invalid inputs, cancellation/reset, and failure states
- core command/event tests with private-data-free Debug output
- Tauri DTO and IPC contract tests whenever snapshot or command shape changes
- wasm checks for pure crates when state/search changes
- local homeserver QA tokens for Matrix-affecting behavior
- browser-headless GUI-operation checks only after Phase B slices begin

QA Batch Z later folds the per-feature evidence into #9/#31.

## Non-Goals

- No GUI implementation in this design.
- No child issue closure from this restructuring alone.
- No vendored SDK patch for backup-wide restore in the MVP path.
- No React-owned product semantics for mention payloads, directory membership,
  moderation permissions, activity ordering, credential health, notification
  decisions, locale fallback, or CJK behavior.
