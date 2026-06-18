# Dogfood Blockers #77–#83 — Completion Design

- Date: 2026-06-19
- Status: Revised after codex review (2026-06-19); pending user review
- Branch: `feat/dogfood-blockers-77-83`
- Issues: #77, #78, #79, #80, #81, #83
- Cross-cutting constraint: issue #87 (behavior-preserving modularization +
  mobile/desktop shared-core seam)

## 1. Context & current state

A prior session began implementing all six issues simultaneously, directly on
`main`, and was interrupted mid-edit. That work-in-progress has been carried to
this branch as commit `89db64d` ("checkpoint … does not build yet"). The diff
base for the eventual codex diff review is `ad1695f` (last commit before the
WIP).

Verified ground truth (not the prior agent's optimistic self-report):

- **Rust does not compile.** `cargo check --workspace` fails (exit 101) with 6
  errors, all in `matrix-desktop-core` from the unfinished #77 crawler wiring:
  1. `unresolved import matrix_desktop_search::AttachmentKind`
  2. `unresolved import matrix_sdk::ruma::api::client::direction`
  3. + 4. non-exhaustive `match` on `SearchCommand` (the two new
     `StartHistoryCrawl` / `StopHistoryCrawl` variants) — in `search.rs`
     `Display`/debug formatting around lines 197/205
  5. `no method named clone` for `MessagesOptions`
  6. `no method named ok` for `&RawValue`
- **TypeScript typecheck passes** (`tsc --noEmit`, exit 0).
- The vendored SDK submodule had only a cosmetic trailing-whitespace diff in
  `redecryptor.rs`; it has been reverted. There is **no vendored-SDK patch
  dependency** in this work.
- #78–#83 are largely implemented but **test coverage is thin**; #77 is broken.

Per-issue starting point (to be verified and completed, not trusted as done):

| Issue | Present in WIP | Main remaining work |
|---|---|---|
| #77 crawler | crawler module, state, command/event variants, reducer, routing | **fix 6 compile errors**, finish wiring, auto-start, settings UI, tests |
| #78 download progress | state enum, action, reducer, event, `MediaAttachment` UI, `mediaUrl.ts` | dedupe verification, retry path, headless tests |
| #79 emoji picker | `EmojiPicker.tsx`, `emojiData.ts`, composer wiring, unit tests | **composer-insertion e2e**, narrow-viewport check |
| #80 media previews/viewer | download state, image render, `mediaUrl.ts` | gallery/viewer real render + nav, blob-URL lifetime, **e2e** |
| #81 member-list entry points | pill→room-info, `joined_members`, 2 e2e | space member-list coverage, entry-point audit |
| #83 per-message state | timestamp, send marks, receipts, re-edit | **e2e for all of it**, compact/hover overlap check |

## 2. Goals / Non-goals

**Goals**

- Bring #77–#83 to dogfood-ready: every issue's acceptance criteria satisfied.
- **Done-bar = all headless tests green** (see §6) plus structural gates
  (typecheck, `cargo check`, wasm-check, secret-scan, IPC-contract) and **two
  codex reviews** (this design, then the final diff).
- New code is shaped per the issue #87 modularization direction (§3).

**Non-goals**

- No real-account / real `matrix.org` testing (deferred by the user). Local
  homeservers (conduit/tuwunel) only.
- **No #87 mechanical refactor of existing giant files** (`state.rs`,
  `reducer.rs`, `App.tsx`, `commands.rs`, …). #87 requires mechanical moves to
  be isolated, behavior-preserving PRs that do not mix with semantic change.
  This work is semantic feature completion; it only ensures **new** code is
  modular and does not worsen the giants.
- No product/UX redesign of the accepted WIP architecture. We complete and
  harden it; we do not re-litigate it.

## 3. Issue #87 alignment principles (applied to new code only)

These are the rules every work package follows so this feature work pre-aligns
with #87 instead of fighting it.

1. **New cohesive units live in their own files** (re-exported from the current
   module), never appended to the giants. Adding a new file + `pub use` is *not*
   the #87 "mechanical move of existing code", so it does not violate #87's
   "don't mix mechanical move with semantic change" rule.
   - Rust: crawler logic stays in `search_crawler.rs`. The new state-type
     clusters move out of `state.rs` into dedicated modules and are re-exported:
     - `crates/matrix-desktop-state/src/state/search_crawler.rs` —
       `SearchCrawlerState`, `SearchCrawlerRoomState`, `SearchCrawlerSettings`,
       `SearchCrawlerSpeed`, `SearchCrawlerFailureKind`.
     - `crates/matrix-desktop-state/src/state/media_download.rs` —
       `TimelineMediaDownloadState`, `MediaTransferProgress`.
     - `state.rs` keeps the `AppState` / `SettingsValues` / `TimelinePaneState`
       fields; callers import the new types via `state::search_crawler` /
       `state::media_download` module paths. Add a compatibility `pub use` only
       where an existing caller demonstrably requires it (avoid widening the
       public surface — see #87 Phase 3). (`AppState`, `reducer.rs`, and
       `action.rs` are *not* split here — that is #87 Phase 2.)
   - React: new UI is extracted into bounded components rather than growing
     `TimelineView.tsx` / `App.tsx`:
     - `MediaAttachment` (timeline card, #78/#80)
     - `MediaViewer` / media gallery (#80)
     - `MessageMeta` (timestamp + send-state mark + read receipts, #83)
     - `EmojiPicker` / `emojiData` / `mediaUrl` already separate (keep).
2. **Domain vs presentation separation** (anticipating #87 Phase 4). Rust-owned
   *domain*: media-download state, crawler state/progress, `joined_members`,
   read receipts, send state. *Presentation* (may be React-local per the
   existing carve-out, since the displayed data is Rust-owned): media
   viewer/gallery open-closed + current index, emoji-picker open, hover/tooltip
   visibility, scroll. Domain **selection** (e.g. active room) stays Rust-owned.
3. **No platform/OS crates in `core`/`state`** (#87 Phase 5). The crawler uses
   only public SDK APIs; media download uses core's existing media path; no
   `keyring`/OS dependency is added to domain crates.
4. **Tests are modular too.** Each feature gets its **own** Playwright spec file
   (`emoji-picker.spec.ts`, `media-attachments.spec.ts`,
   `timeline-message-state.spec.ts`, `member-list.spec.ts`,
   `search-crawler.spec.ts`) instead of growing `basic-operations.spec.ts`.
   This also removes cross-agent file contention (§5).
5. **Public-surface discipline** (#87 Phase 3). Export new types through their
   module path (`state::search_crawler`, `state::media_download`); do not add
   broad `state.rs` / `lib.rs` re-exports beyond what existing callers
   demonstrably need. Fewer re-exports now = less to narrow at #87 Phase 3.

## 4. Per-issue design & completion scope

Contracts already chosen by the WIP are kept unless noted. "Mirror" means: keep
the Rust state, Tauri `dto.rs`, TypeScript `types.ts`, `coreEvents.ts`, and
`coreEvents.generated.json` in sync (and the DTO/IPC contract tests).

### #77 — Search history crawler (broken → complete)

**Kept contracts.** `SearchCommand::{StartHistoryCrawl, StopHistoryCrawl}`;
`SearchEvent::{HistoryCrawlProgress, HistoryCrawlCompleted, HistoryCrawlFailed}`;
`AppState.search_crawler: SearchCrawlerState` keyed by room, with
`SearchCrawlerRoomState ∈ {Idle, Running{processed, indexed}, Completed{indexed},
Failed{kind}}`; `SettingsValues.search_crawler: SearchCrawlerSettings {
speed ∈ {Standard, Fast, Slow, Paused}, include_media_captions,
include_filenames }`.

**Privacy of failures (revised per review, finding 1).** The mirrored failure
carries only a coarse Rust-owned `kind`
(`RoomNotFound | Sdk | Decryption | IndexUnavailable`), rendered via an i18n
catalog key. The raw error string is **removed from the reducer state and the
DTO** (the WIP's `Failed{message}` field is dropped); the detail is logged
internally, redacted, and never crosses the Tauri/TypeScript boundary or appears
in QA output. A test must assert that no raw SDK error text or Matrix identifier
reaches the snapshot DTO.

**Compile fixes (WP1).**

- `AttachmentKind`: export it from `matrix-desktop-search`'s public API (or use
  the existing search document type) rather than importing a private/absent
  symbol.
- `Direction`: import the correct ruma path for backward paging
  (e.g. `matrix_sdk::ruma::api::Direction::Backward` via `MessagesOptions`).
- non-exhaustive `SearchCommand` matches: add arms for the two new variants in
  the `search.rs` formatter(s).
- `MessagesOptions` is not `Clone`: build a fresh `MessagesOptions` per page
  (carry the `from` token forward) instead of cloning.
- `&RawValue`: deserialize via `serde_json::from_str(raw.get())` instead of a
  non-existent `.ok()`.

**Behavior.** Page backward through `/rooms/{id}/messages` in bounded batches
(200 / 100 / 50 by speed) with throttle (0 / 100 / 500 ms); `Paused` disables.
Index event text plus, per settings, media captions / filenames / metadata —
**never fetch attachment bytes**. Decrypt via public SDK API before indexing;
skip UTD events. Progress/Completed/Failed projected into
`AppState.search_crawler`.

**Auto-start (new, approved by user; gated per review, finding 2).** Auto-start
is gated behind the persisted Rust-owned `SearchCrawlerSettings.speed`: when
crawling is enabled (`speed != Paused`) and a *joined* room is eligible, enqueue
an **idempotent** background crawl — never restart a room already `Running` or
`Completed`. Manual `StartHistoryCrawl` / `StopHistoryCrawl` remain for explicit
control.

Default: `speed = Standard` (auto-on) for the current **local-only / pre-dogfood
phase** — faithful to the user's "auto-start" instruction and safe because only
local test homeservers are used. Codex flagged that a persisted auto-on default
is risky once real accounts exist (large histories decrypted/indexed, server
load). Mitigation + **hard gate**: the default is a single Rust-owned persisted
value and **must be revisited before any real-account / matrix.org support**
(switch to default `Paused` with a first-run opt-in / migration). QA scenarios
set `speed` explicitly rather than relying on the default.

**Settings-change invalidation (guarded transitions, per review, finding 3).**
The crawler is a guarded state machine over settings changes, not only over room
discovery:
- `Paused → active` (any non-`Paused` speed): enqueue **all** currently-known
  eligible joined rooms (not just rooms observed after the toggle), so enabling
  crawling backfills the existing room list.
- `active → Paused`: stop in-flight crawls; keep `Completed` markers.
- A **content-affecting** settings change (`include_media_captions` or
  `include_filenames` toggled): invalidate affected `Completed` rooms back to an
  eligible/`Idle` state so their indexes are rebuilt with the new inclusion
  semantics; a pure `speed` change does **not** invalidate completed indexes.
These transitions are Rust-owned and covered by reducer/state-machine tests; if
the reducer gains new guarded transitions, keep the normative state-machine
diagram in `docs/architecture/state-machine.md` in sync.

**Settings UI (WP3).** A Search section: speed selector (incl. Paused = off),
include-captions and include-filenames toggles, and per-room status
(Idle/Running n/indexed/Completed/Failed) with start/stop. Renders Rust-owned
state only.

**Tests.** Crawler unit tests: no attachment bytes fetched, metadata indexed,
edits skipped, throttle honored, failure surfaced as coarse `kind` only (assert
no raw SDK error / Matrix id crosses the DTO). Reducer/state-machine tests for
the guarded transitions above (enable enqueues all known eligible rooms;
content-setting change invalidates completed rooms; idempotent no-restart). A
headless-core-qa scenario (extend `search` or add `search_crawler`) proving
backfill paging, no-media-byte fetch, throttle, and visible failure —
**token-only** output (e.g. `crawl_backfill=ok`, `crawl_no_media_bytes=ok`,
`crawl_throttle=ok`, `crawl_failure=ok`); no room IDs, event IDs, bodies, tokens,
or raw SDK errors.

### #78 — Attachment download pending/progress

**Kept contracts.** `TimelineMediaDownloadState ∈ {NotRequested,
Pending{progress?}, Ready{source_url, width, height, mime_type}, Failed{kind}}`;
`TimelinePaneState.media_downloads: BTreeMap<event_id, …>`;
`TimelineEvent::MediaDownload{Progress, Completed, Failed}`;
`AppAction::MediaDownloadUpdated`.

**Completion.** Click → `Pending` immediately; determinate progress when
bytes/total known, indeterminate fallback otherwise; **dedupe** (no second
download for the same `event_id` while one is `Pending`/`Ready`); `Failed`
offers retry and never leaves a permanent spinner; encrypted "preparing" must
not look idle. Relocate the state types to `state/media_download.rs`; the
`MediaAttachment` component owns the card.

**Tests.** Vitest component states + `media-attachments.spec.ts` (pending,
progress, success, failure, retry, dedupe). Linux GUI `local-media` lane covers
the click→download path in the real WebView.

### #80 — Real media previews + viewer

**Design.** Reuse the #78 download state; render `Ready.source_url` through
`mediaUrl.ts` (`convertFileSrc`). Gallery/viewer open-closed + index are
presentation state. Real `<img>` previews in timeline cards when `Ready`;
gallery + viewer render real media with next/previous; explicit
pending/preparing/ready/failed; **bounded blob-URL lifetimes** (revoke on
unmount / when superseded); a11y labels + keyboard nav in the viewer; opening
the gallery must **not** download unrelated media; icon-only stays only for
non-previewable types or failures. New `MediaViewer` / gallery component(s).

**Tests.** `media-attachments.spec.ts` (or a sibling) pins timeline preview
render, gallery open, viewer next/prev, loading, failure. `local-media` GUI lane
already opens gallery + viewer end-to-end.

### #81 — Member-list entry points

**Design.** `RoomSummary.joined_members` is Rust-owned. Audit each entry point
and make it dispatch a Rust-owned command / open the expected surface:
room-header member pill → open Room info; Room info members section; Room info
"People" → scroll to members; Space info members section + "Members" button.
Room vs space member lists stay distinct (rooms have moderation CRUD; spaces are
read-only member display). Dead/ambiguous controls are wired, or disabled/removed
with an explicit reason. Disabled/loading/error states are explicit.

**Tests.** `member-list.spec.ts` clicks every entry point and asserts the
expected Rust-owned command / state transition; room and space covered
independently.

### #83 — Timeline per-message state

**Kept contracts (Rust-owned).** `TimelineItem.{timestamp_ms, send_state,
is_edited, can_edit}` and the existing read-receipt DTOs
(`LiveReadReceipt` / `LiveEventReceiptSummary`). React must not invent
send/read/edit state.

**Completion.** Visible timestamp on normal message rows; compact send-state
marks including a **sent/success** mark for own messages (currently
intentionally hidden); failure/pending stay actionable (retry/cancel/delete);
read receipts verified for one reader, multiple readers, avatar fallback,
overflow `+N`, and tooltip; edited messages re-editable when `can_edit`; no text
overlap in compact/hovered states. Extract a `MessageMeta` subcomponent.

**Tests.** `timeline-message-state.spec.ts`: timestamp display; sending / sent /
not-sent / cancelled / failed marks; 1 reader / many readers / avatar fallback /
overflow; edit → edited marker → edit again → saved second edit. (`ad1695f`
already added cancelled-state regression coverage.)

## 5. Work decomposition (Sonnet implements) & contention control

The user directs that **implementation itself is delegated to Sonnet**. Opus
owns design, shared-surface integration, commits, codex reviews, and
verification; Sonnet output is verified, never accepted blindly. No two agents
edit the same hot file concurrently.

- **WP1 — backend + compile + full mirror (Sonnet, solo, first).** Owns all
  shared **Rust** hot files (`state.rs`/new state submodules, `action.rs`,
  `reducer.rs`, `command.rs`, `event.rs`, `search.rs`, `account.rs`,
  `sdk/src/lib.rs`) and `search_crawler.rs`, **and the complete DTO/IPC mirror
  for the backend types it adds**: `dto.rs`, `types.ts`, `coreEvents.ts`,
  `coreEvents.generated.json`, plus the DTO + IPC-contract tests. Fix the 6
  errors, finish #77 incl. gated auto-start + invalidation transitions, relocate
  new state types into submodules, crawler unit tests + headless-core-qa crawler
  scenario. **Exit criterion: `cargo build`, `cargo test --workspace`,
  `typecheck`, and the IPC/DTO contract tests all green.** WP1 is the single
  serialized owner of the DTO mirror; **all dependents are blocked until it
  lands** (resolves the review's mirror-ownership conflict, finding 4).
- **WP2 — timeline lane (Sonnet, after WP1).** #78 + #80 + #83. Sole owner of
  `TimelineView.tsx`, the new timeline subcomponents, `mediaUrl.ts`, timeline
  CSS, and the timeline e2e specs. Single owner avoids `TimelineView.tsx`
  contention between the three issues.
- **WP3 — composer/panels lane (Sonnet, parallel with WP2).** #79 + #81 + #77
  Settings UI. Owns `App.tsx` composer/pill regions, `EmojiPicker`,
  `RoomInfoPanel` / `SpaceInfoPanel`, the Settings Search section, and the
  emoji/member-list e2e specs.
- **Opus integration gate (between WP1 and WP2/WP3).** After WP1 lands the full
  backend mirror, Opus runs a single gate that pre-places only the
  **front-end-only** shared surfaces WP2/WP3 need: `i18n/messages.ts` keys and
  any presentation-only `types.ts` additions not already supplied by WP1. The
  DTO mirror (`dto.rs`, `coreEvents*`, domain `types.ts`) is **not** touched here
  — WP1 already owns it. WP2/WP3 then edit only their disjoint component files +
  their own spec files.

Ownership map (single owner per file, serialized): Rust hot files + full DTO
mirror (`dto.rs` / `coreEvents*` / domain `types.ts`) → WP1; FE-only shared
surfaces (`messages.ts`, presentation `types.ts`) → Opus gate after WP1;
`TimelineView.tsx` + timeline subcomponents → WP2; `App.tsx` / panels / Settings
→ WP3. No file has two concurrent owners.

## 6. Done-bar — all headless tests

Environment verified to support the full stack: conduit/tuwunel/headless-core-qa
binaries in `/tmp/matrix-desktop-local-qa-bin`, plus `tauri-driver`, `Xvfb`,
`/usr/bin/WebKitWebDriver`, and `codex`.

Gates and tests (run with `PATH=/tmp/matrix-desktop-local-qa-bin:$PATH` where a
local homeserver is needed):

- `cargo test --workspace`
- `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` (DTO + IPC
  contract)
- `npm --prefix apps/desktop run typecheck`
- `npm --prefix apps/desktop run test` (Vitest) and `npx playwright test`
- `npm --prefix apps/desktop run qa:wasm-check`
- `npm --prefix apps/desktop run qa:secret-scan`
- headless-core-qa scenarios: crawler/search (new/extended), media — token-only
- Linux GUI virtual-display lanes: `local-media` (and any new/relevant lane for
  member-list / message-state / emoji if added)

Reporting rule: pass/fail stated honestly with evidence; **no skipped or
weakened assertions**; all QA output private-data-free.

## 7. Risks & mitigations

- **`TimelineView.tsx` contention** across #78/#80/#83 → single WP2 owner +
  subcomponent extraction.
- **Shared-mirror drift** (Rust state ↔ dto.rs ↔ types.ts ↔ coreEvents{,.json})
  → WP1 is the single serialized owner of the full mirror and lands before
  dependents; the `core_event_wire_format…` IPC-contract test + the `dto.rs`
  serialization-contract test gate it.
- **headless-core-qa flakiness** (documented in AGENTS.md) → bounded `SyncOnce`
  per the live-signals/e2ee notes; verify both SyncService and legacy legs where
  relevant.
- **Crawler auto-start load** → bounded batches + throttle + idempotent
  start; local servers only.
- **Playwright reply-spec known flake** (`basic-operations.spec.ts:81`) → run
  affected specs isolated / `--workers=1`; new specs are independent files.

## 8. References

Issues #77–#83 and #87; `REPOSITORY_RULES.md`;
`docs/architecture/{overview,state-machine,i18n}.md`;
`docs/policies/engineering-rules.md`; `AGENTS.md`. Coordinate naming with the
Koushi rename (#82) only at #87 Phase 6 — out of scope here.
