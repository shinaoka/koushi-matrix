# Issue 118 Completion Design

## Goal

Complete GitHub issue #118 end to end: timeline scrollback must feel fast after selection and restart, timeline events and room position must persist safely, room-list and member-list work must scale to large accounts and large rooms, search-indexing settings must present accurate progress, and the remaining #116-derived structural risks must have explicit QoS boundaries.

## Current Findings

The #116 blocker was fixed by making room selection observable, removing the member-avatar thumbnail flood from the snapshot planner, moving avatar thumbnail fetches behind cache-first bounded workers, requesting only visible avatars, preserving DM avatar thumbnails across projection, and making `ensure_timeline_subscribed` idempotent.

Issue #118 is the follow-up umbrella. The code still has several unresolved high-impact items:

- `TimelineSettings::default()` leaves `auto_load_older_messages` as `false`, so existing and new settings default to top-only pagination instead of 100-items-ahead prefetch.
- The Matrix SDK store is configured with a cache path and the same encrypted SQLite config key, but Koushi does not subscribe the SDK event cache, so sync/latest-event paths that require `client.event_cache().has_subscribed()` stay inactive.
- `NavigationState` persists active space and active room, but no per-room timeline anchor. Restart restores the room, not the user's in-room reading position.
- `matrix_room_list_snapshot_from_rooms` still calls `collect_active_member_profiles` for every joined room on every room-list snapshot. That loads all active members and creates O(total members across rooms) reprojection cost.
- Search crawler pause still stops actor work without reducer-authoritatively moving running rooms out of `Running`.
- Issue #117 remains open for renderable thumbnail cache encryption. Koushi still writes redundant plaintext files under `avatar_thumbnails` and `link_preview_thumbnails` and returns `file://` URLs to the WebView.
- The crawl-speed selection, separate room-index panel, and bounded room-index list have already been implemented and should be marked complete after tests.

## Non-Goals

Do not redesign the whole actor system before closing #118. Do not replace the Matrix SDK timeline/event-cache APIs. Do not make search indexing or media caches more permissive with private data. Do not introduce a new frontend state store.

## Issue #118 Checklist Mapping

The implementation must close the issue's checkboxes with this mapping:

| Issue #118 checkbox | Slice |
| --- | --- |
| Prefetch older messages by default | 1 |
| Persist downloaded timeline events across restart, with encrypted-at-rest prerequisite | 2 |
| Persist per-room scroll/read position across restart | 3 |
| Reduce room-list re-projection churn | 4 |
| Member-list virtualization + per-visible-member avatars | 5 |
| Encrypt the avatar/media thumbnail cache (#117) | 6 |
| Full user-intent QoS lane separation / reducer-authoritative intent lifecycle | 8 |
| Clearer search-indexing progress UI | 7 |
| Crawl-speed buttons show selected/default | 7 verification only, because implementation appears present |
| Pause crawler leaves rooms showing running | 7 |
| More info should show per-room crawl detail | 7 |
| Move Room index status to its own panel | 7 verification only, because implementation appears present |
| Make the long room list scrollable | 7 verification only, because implementation appears present |

No checkbox is considered closed by intent alone. Each row needs automated verification or a private-data-free manual verification note.

## Approach

Use a staged completion plan with small, reviewable boundaries:

1. **Fast scrollback defaults**: flip the default and legacy backfill for `timeline.auto_load_older_messages` to `true`.
2. **Encrypted event-cache persistence**: prove the cache SQLite store uses the existing local encryption key, subscribe the SDK event cache during client/session setup, and surface failures as diagnostics rather than silently continuing.
3. **Per-room scroll anchors**: persist room-local reading anchors in `NavigationState`, update them from the UI scroll viewport, and restore via focused context or timeline anchor APIs when reopening a room after restart.
4. **Room-list reprojection scale**: remove full member-list loads from the hot room-list snapshot path. Use cheap joined-member counts for room summaries, direct account-data targets for DMs, and lazy member profile loading only in member-list/room-info flows.
5. **Member-list and visible avatars**: virtualize large member lists and request member avatars only for visible rows.
6. **Renderable thumbnail cache encryption**: close #117 by removing redundant plaintext thumbnail files and serving sensitive thumbnails only through an encrypted-at-rest, in-memory-decrypt render path.
7. **Search crawler accuracy**: make pause a reducer-visible state transition, improve the progress summary, and keep the existing separate/scrollable panel behavior.
8. **QoS guardrails**: separate or prioritize user-intent and foreground work enough that future background floods cannot block room selection again. This can be implemented as bounded intent/foreground queues and explicit drop/coalesce rules for background lanes rather than a full actor rewrite.
9. **Verification and issue hygiene**: run targeted Rust, TypeScript, and e2e tests; update #118 after every checkbox has evidence.

## Data Model

### Timeline Settings

`TimelineSettings` keeps the existing `auto_load_older_messages: bool` field. Its default becomes `true`. Deserialization must treat a missing `timeline` object or missing `auto_load_older_messages` field in legacy JSON as `true`, while preserving an explicitly stored `false`. The prefetch behavior is active-room foreground work only and must route through the foreground QoS lane, not a background all-room prefetch.

### Event Cache

`MatrixClientStoreConfig` appears to build a `SqliteStoreConfig` with `MatrixClientStoreKey` and pass a cache path to `sqlite_store_with_config_and_cache_path`. The current vendored SDK appears to clone that config and change only the path for event-cache/media stores. This is a privacy-critical SDK-internal behavior and must be verified, not asserted.

Acceptance requires evidence from tests that:

- event-cache content written to the SQLite cache path is not readable plaintext at the byte level;
- opening the event-cache store with a wrong or empty key fails or cannot read the stored events;
- SDK media-store content used for cached media follows the same encrypted-store property;
- `event_cache_store_encrypted=true` is emitted only after the implementation has passed those tests, not merely because a config field exists.

Event-cache subscription belongs in the SDK/session setup layer, not in UI commands. It must run once per store-backed Matrix client before sync-dependent work assumes the event cache is active. The implementation must distinguish "subscribed" from "persisted": `event_cache().subscribe()` activates cache listeners, but #118 completion also requires a kill-and-relaunch verification that events written before process exit are loaded from encrypted storage after restart.

The result should be observable in diagnostics with private-data-free fields:

- `event_cache_subscribe=ok`
- `event_cache_subscribe=failed reason_class=<class>`
- `event_cache_store_encrypted=true`

Failure to subscribe should not crash login, but it must be visible in diagnostics and the timeline restart-speed acceptance test must fail while subscription or persistent reload is disabled.

### Navigation Anchors

Extend `NavigationState` with:

```rust
#[serde(default)]
pub room_scroll_anchors: BTreeMap<String, TimelineScrollAnchor>,
```

`TimelineScrollAnchor` stores private identifiers already used locally:

```rust
pub struct TimelineScrollAnchor {
    pub event_id: String,
    pub offset_px: i32,
    pub updated_at_ms: u64,
}
```

Navigation is persisted through the existing per-account encrypted navigation store, not `settings/settings.json`. Room IDs and event IDs are acceptable only in that encrypted local state. They must never be copied into the non-secret settings store or diagnostic messages.

Keep anchors per room with a bounded LRU policy. The default cap is 200 rooms, evicting the oldest `updated_at_ms` entries when saving. Anchors older than 30 days are ignored for restore and can be pruned on save.

`offset_px` is the offset of the viewport anchor relative to the anchored event's top, not an absolute scroll position. Restore first anchors by event ID, then applies `offset_px` best-effort so window size, font, and message-height changes do not make the stored value authoritative.

When selecting a room, restore the anchor only if it still belongs to that room and is within the staleness window. Prefer the live timeline when the event is already loaded or quickly available from event-cache storage. The SDK's focused-event timeline path (`TimelineBuilder::with_focus(TimelineFocus::Event { target: event_id, ... })`) is a separate event-focused controller and, in the current architecture, is not yet proven to hand the anchor back to the live room actor after a bootstrap. Until that bridge exists, treat the focused bootstrap requirement as unresolved and fall back to the current live-edge/read-marker behavior when the event is unavailable or cannot be shown in the live timeline.

### Room Summary Without Full Members

Room summaries should not contain every active member. Replace hot-path `active_user_ids` usage with:

- `joined_members`: an SDK cheap count such as `joined_members_count` or a state-store count API.
- DM counterpart IDs: direct account data first; for rooms that are marked DM without direct account-data targets, use a lazy fallback that reads only enough member identity to resolve the counterpart, not all profiles for all rooms.
- `snapshot.user_profiles`: only profiles already required by direct DM counterpart resolution or explicitly requested visible/member flows.

The hot room-list snapshot target is O(number of rooms) plus the small number of DM counterpart lookups, not O(total active members).

### Renderable Thumbnail Cache

SDK event-cache/media stores can be encrypted through `SqliteStoreConfig::key`, but Koushi's renderable thumbnail files are separate plaintext artifacts. Closing #117 means removing or encrypting these artifacts:

- Avatar thumbnails: stop writing `data_dir/avatar_thumbnails/<hash>.<ext>` as plaintext in `download_avatar_thumbnail`.
- Link preview thumbnails: stop writing `data_dir/link_preview_thumbnails/<hash>.<ext>` as plaintext in `download_preview_image`.
- Media downloads: keep user-requested downloads explicit, but do not use the renderable cache directory for background thumbnails unless the bytes are encrypted at rest.

Prefer a custom Tauri protocol or command that serves bytes from the encrypted SDK media store or an encrypted Koushi cache and streams decrypted bytes in memory.
Plaintext `file://` is not an acceptable renderable path for sensitive thumbnails. Encrypted bytes cannot be rendered by `<img src="file://...">`, and renderable `file://` bytes are plaintext. The acceptable path is an in-memory decrypt-and-serve custom protocol or command-backed blob URL whose backing plaintext is not persisted. If a transition period is needed, it can only serve genuinely non-sensitive assets.

Add one-time cleanup that removes old plaintext files under `avatar_thumbnails/` and `link_preview_thumbnails/` after the encrypted serving path is available. The cleanup must not delete explicit user-requested downloads.

To avoid regressing #116, decrypted thumbnail serving must remain cache-first and bounded:

- no synchronous decode/decrypt in the AccountActor hot loop;
- concurrent decrypt/serve work is bounded;
- visible-range requests remain the only automatic avatar requests;
- large-account verification includes avatar render latency under background load.

### Search Crawler State

The reducer remains authoritative for user-visible crawler state. When settings transition from active speed to `Paused`, all `Running` rooms become `Queued` or a new explicit `Paused` state. Prefer adding `Paused` only if the UI needs to distinguish paused from queued; otherwise convert `Running` to `Queued` and make the summary text say indexing is paused.

The progress summary should use user-facing language:

- Active: `Indexing message history... N of M rooms`
- Paused: `Message history indexing is paused. N of M rooms indexed`
- Error detail: keep failed/running/queued counts in an expandable or detail area.

## Frontend Design

The first screen remains the application shell. Settings changes are contained to the existing User Settings panel.

Timeline anchor capture stays in the timeline UI layer because the DOM viewport knows the visible item and pixel offset. The UI sends a small command or setting patch when the stable top/center anchor changes, throttled to avoid persistence churn. The Rust state owns persistence and restoration.

Member lists should render with virtualization for large rooms. Use existing component patterns and CSS density; do not add decorative cards. Rows keep stable heights so avatar loading cannot shift layout.

## Backend Design

### Event Cache Lifecycle

Add a `MatrixClientSession::enable_event_cache` or SDK helper around `client.event_cache().subscribe()`. Call it after the store-backed client is built and before long-running sync/timeline work starts. Make it idempotent by checking `has_subscribed()` first.

### QoS Lanes

Close #118 with pragmatic QoS guardrails:

- User intents such as `SelectRoom`, `SelectSpace`, composer send, and open focused context must use reliable send, bounded wait, request IDs, and terminal lifecycle outcomes.
- Foreground room work such as active timeline subscription and visible avatar requests must be prioritized ahead of background projection/search/media work.
- Background work such as inactive room-list enrichment, search crawling, and non-visible media/avatar requests may be coalesced, latest-wins, or dropped on full queues if the product state remains recoverable.

Do not block user intent on background member/profile/media fetches.

## Error Handling

All formerly silent background failures that affect #118 completion should become diagnostics:

- event-cache subscription failure
- scroll-anchor restore failure class
- room-list snapshot member-count/profile fallback failure class
- renderable thumbnail cache encryption/serve failure class
- crawler pause transition mismatch

Do not log message bodies, room names, room IDs, event IDs, user IDs, access tokens, or event content in diagnostics. Counts, booleans, request IDs, and reason classes are acceptable.

## Testing

Required tests before closing #118:

- Rust state tests proving timeline settings default/backfill to `true`.
- Rust SDK/core tests proving event-cache store encryption at the byte/wrong-key level and proving `event_cache().subscribe()` is called idempotently.
- Restart verification that kills and relaunches the process/client, then proves timeline events are loaded from persistent event-cache storage rather than warm in-memory state.
- Rust navigation store tests proving anchors encrypt/persist/load with legacy navigation compatibility.
- Rust SDK tests proving room-list snapshot does not call full `room.members(RoomMemberships::ACTIVE)` in the hot path.
- Rust SDK/core tests proving one-to-one DM rooms still resolve labels and avatars via local alias, direct account-data counterpart, and profile fallback without regressing to raw MXID labels.
- Rust/core or Tauri tests proving avatar/link-preview thumbnails are not persisted as plaintext renderable files.
- Rust reducer/core tests proving a transition to `Paused` moves all `Running` crawler rooms out of `Running`, with no room left `Running` while the actor is stopped.
- React/component tests proving crawler speed remains selected, pause summary is accurate, progress text is user-facing, and the room-index panel remains scrollable.
- Timeline UI tests proving anchor capture dispatches throttled updates and restore request is issued on room activation.
- Targeted e2e or harness test for a large account shape proving selecting a DM reaches first timeline paint within a concrete budget while room-list/member/avatar/search background lanes are saturated. The default budget is under 1 second in the deterministic harness and under 2 seconds in real-account manual QA, measured from intent submission to first matching timeline item render.

## Acceptance Criteria

Issue #118 is complete when:

- Every checkbox in issue #118 is implemented, tested, and backed by either automated verification or a private-data-free manual verification note.
- `auto_load_older_messages` defaults to `true` for new and legacy settings.
- SDK event cache persistence is enabled and protected by byte-level and wrong-key evidence that the event-cache SQLite store is encrypted.
- #117 is closed with direct evidence that renderable avatar/link-preview thumbnail bytes are no longer plaintext on disk.
- Restart restores active room and per-room reading anchor when possible.
- Hot room-list reprojection is O(rooms), not O(total members).
- Large member lists are virtualized and visible avatars are requested only for visible members.
- Search crawler pause and progress UI reflect actual state.
- User-intent/foreground/background work have explicit priority/drop semantics and automated large-account tests that would catch another #116-style background flood.
- The implementation has been reviewed by the main agent and Claude CLI.

## Implementation Slices

The implementation should be delegated in slices with disjoint primary ownership:

1. Settings default and legacy backfill.
2. Event-cache encryption proof and subscription lifecycle.
3. Navigation anchor persistence and timeline UI capture/restore.
4. Room-list hot-path member-count/profile rewrite.
5. Member-list virtualization and visible avatar requests.
6. Renderable thumbnail cache encryption / plaintext file removal.
7. Search crawler pause/progress cleanup.
8. QoS guardrail tests and diagnostics.
9. Final issue update and regression verification.

Each slice should include tests and a short self-review. The main agent reviews all changes, then Claude CLI performs an independent review before the next slice is accepted.
