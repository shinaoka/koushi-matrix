# Issue 118 Completion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete every checkbox in GitHub issue #118, including event-cache persistence and #117 thumbnail-cache encryption.

**Architecture:** Implement in isolated slices with tests first. Keep user-intent and active-room work ahead of background work, keep private identifiers out of diagnostics, and prove encrypted persistence with behavioral evidence instead of config assumptions.

**Tech Stack:** Rust 2024, Tauri v2, React/TypeScript, Matrix Rust SDK, Vitest, Playwright, Cargo tests.

---

## File Structure

- `crates/koushi-state/src/state/settings.rs`: timeline default/backfill.
- `crates/koushi-state/tests/settings_state.rs`: settings default and legacy JSON tests.
- `crates/koushi-sdk/src/lib.rs`: event-cache enablement, room-list snapshot hot path, SDK-level tests.
- `crates/koushi-core/src/account.rs`: call SDK event-cache enablement after store-backed restore; avatar thumbnail plaintext removal.
- `crates/koushi-core/src/store.rs`: encrypted navigation anchors and plaintext thumbnail cleanup helpers.
- `crates/koushi-state/src/state/navigation.rs`: `TimelineScrollAnchor` data model.
- `crates/koushi-state/src/action.rs`, `crates/koushi-state/src/reducer/navigation.rs`: anchor update action/reducer.
- `apps/desktop/src-tauri/src/commands/navigation.rs`, `apps/desktop/src-tauri/src/commands/mod.rs`: anchor update command builders if the UI sends anchors directly.
- `apps/desktop/src/components/TimelineView.tsx`: capture visible anchor and request restore.
- `apps/desktop/src/components/RoomInfoPanel.tsx`: member list virtualization and visible avatar requests.
- `apps/desktop/src/components/UserSettingsPanel.tsx`: search crawler progress/pause/detail UI.
- `crates/koushi-state/src/reducer/settings.rs`, `crates/koushi-state/src/reducer/search.rs`, `crates/koushi-core/src/search.rs`: crawler pause state correctness.
- `docs/superpowers/specs/2026-06-22-issue-118-completion-design.md`: source spec.

## Task 1: Timeline Scrollback Default

**Files:**
- Modify: `crates/koushi-state/src/state/settings.rs`
- Modify: `crates/koushi-state/tests/settings_state.rs`
- Check generated/mocks after tests: `apps/desktop/src/test/tauriIpcMock.ts`, `apps/desktop/src/test/appHarnessMain.tsx`, focused tests that hard-code `auto_load_older_messages: false`

- [x] **Step 1: Write failing Rust tests**

Add tests in `crates/koushi-state/tests/settings_state.rs`:

```rust
#[test]
fn timeline_auto_load_older_messages_defaults_to_true() {
    let values = koushi_state::SettingsValues::default();
    assert!(values.timeline.auto_load_older_messages);
}

#[test]
fn missing_timeline_settings_backfill_auto_load_to_true() {
    let json = r#"{
      "locale": {"language_tag": null, "text_direction": "auto"},
      "appearance": {"theme": "system"},
      "typography": {"font_family": "system", "font_size": 14, "code_block_wrap": true},
      "keyboard": {"send_message": "enter"},
      "notifications": {"enabled": true},
      "display": {"show_redacted": false},
      "media": {"inline_image_previews": true}
    }"#;
    let values: koushi_state::SettingsValues = serde_json::from_str(json).unwrap();
    assert!(values.timeline.auto_load_older_messages);
}

#[test]
fn explicit_false_auto_load_older_messages_is_preserved() {
    let json = r#"{
      "locale": {"language_tag": null, "text_direction": "auto"},
      "appearance": {"theme": "system"},
      "typography": {"font_family": "system", "font_size": 14, "code_block_wrap": true},
      "keyboard": {"send_message": "enter"},
      "notifications": {"enabled": true},
      "display": {"show_redacted": false},
      "media": {"inline_image_previews": true},
      "timeline": {"auto_load_older_messages": false}
    }"#;
    let values: koushi_state::SettingsValues = serde_json::from_str(json).unwrap();
    assert!(!values.timeline.auto_load_older_messages);
}
```

- [x] **Step 2: Run failing tests**

Run:

```bash
cargo test -p koushi-state timeline_auto_load_older_messages_defaults_to_true missing_timeline_settings_backfill_auto_load_to_true explicit_false_auto_load_older_messages_is_preserved
```

Expected before implementation: at least the default/backfill tests fail.

- [x] **Step 3: Implement the default**

Change `TimelineSettings` to use a custom default:

```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelineSettings {
    #[serde(default = "default_true")]
    pub auto_load_older_messages: bool,
}

impl Default for TimelineSettings {
    fn default() -> Self {
        Self {
            auto_load_older_messages: true,
        }
    }
}
```

Do not rewrite explicit persisted `false`.

- [x] **Step 4: Update frontend test fixtures**

Only change fixtures that represent a default snapshot. Leave tests that intentionally cover disabled prefetch at `false`.

- [x] **Step 5: Verify**

Run:

```bash
cargo test -p koushi-state settings_state
cd apps/desktop && npm run typecheck
```

Expected: both pass.

- [x] **Step 6: Commit**

```bash
git add crates/koushi-state/src/state/settings.rs crates/koushi-state/tests/settings_state.rs apps/desktop/src/test apps/desktop/src/domain apps/desktop/src/components
git commit -m "fix: enable timeline prefetch by default"
```

## Task 2: Event Cache Encrypted Persistence

**Files:**
- Modify: `crates/koushi-sdk/src/lib.rs`
- Modify: `crates/koushi-core/src/account.rs`
- Test: `crates/koushi-sdk/src/lib.rs` unit/source tests or `crates/koushi-sdk/tests/event_cache_persistence.rs`
- Test: `crates/koushi-core/src/account.rs` source guard if a full SDK integration test is too heavy

- [x] **Step 1: Write tests for encryption evidence and lifecycle**

Add tests that prove these concrete properties:

```rust
#[tokio::test]
async fn event_cache_sqlite_store_is_not_plaintext_and_rejects_wrong_key() {
    let sentinel = "issue118-event-cache-sentinel";
    let correct_key = [7_u8; 32];
    let wrong_key = [9_u8; 32];

    // Create a keyed SqliteEventCacheStore under a temp cache path.
    // Write one synthetic room event containing `sentinel` through the
    // event-cache store API.
    // Drop the store, read every file under the cache path as bytes, and assert
    // `sentinel.as_bytes()` is absent from the raw bytes.
    // Reopen the same cache path with `wrong_key` and assert the stored event
    // cannot be read.
}

#[tokio::test]
async fn enable_event_cache_is_idempotent() {
    // Build a store-backed MatrixClientSession with temp encrypted store paths.
    // Assert `client.event_cache().has_subscribed()` is initially false.
    // Call `enable_event_cache(&session)` twice.
    // Assert the first call returns Enabled, the second AlreadyEnabled, and
    // `has_subscribed()` is true.
}
```

If direct sentinel insertion is easier through `client.event_cache().for_room(...).await`, use the SDK's event-cache room storage tests as reference.

- [x] **Step 2: Add the helper**

In `crates/koushi-sdk/src/lib.rs` add:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MatrixEventCacheStatus {
    AlreadyEnabled,
    Enabled,
}

pub async fn enable_event_cache(
    session: &MatrixClientSession,
) -> Result<MatrixEventCacheStatus, MatrixEventCacheError> {
    let cache = session.client().event_cache();
    if cache.has_subscribed() {
        return Ok(MatrixEventCacheStatus::AlreadyEnabled);
    }
    cache.subscribe().map_err(|error| MatrixEventCacheError::Sdk(error.to_string()))?;
    Ok(MatrixEventCacheStatus::Enabled)
}
```

Define `MatrixEventCacheError` without logging private data.

- [x] **Step 3: Call it after store-backed restore**

In `crates/koushi-core/src/account.rs`, call the helper only on store-backed sessions:

```rust
match koushi_sdk::enable_event_cache(&store_backed).await {
    Ok(status) => {
        self.emit(CoreEvent::Diagnostic(/* private-data-free event or existing diagnostic path */));
    }
    Err(error) => {
        self.emit(CoreEvent::Diagnostic(/* reason_class only */));
    }
}
```

Apply this in login restore, restore last session, switch account, and soft logout reauth paths after `restore_into_store` succeeds and before sync starts.

- [x] **Step 4: Add kill/relaunch verification path**

Extend an existing headless or SDK smoke test path to:

1. create a store-backed client;
2. enable event cache;
3. receive or insert a known event through the SDK cache;
4. drop the client;
5. restore a new client from the same store;
6. prove the event is loaded from storage.

Skip real message bodies in diagnostics; test fixtures may use synthetic content.

- [x] **Step 5: Verify**

Run:

```bash
cargo test -p koushi-sdk event_cache
cargo test -p koushi-core event_cache
```

Expected: new tests pass and no private identifiers appear in logs/assertion messages.

- [x] **Step 6: Commit**

```bash
git add crates/koushi-sdk/src/lib.rs crates/koushi-core/src/account.rs crates/koushi-sdk/tests crates/koushi-core/tests
git commit -m "feat: enable encrypted event cache persistence"
```

## Task 3: Per-Room Timeline Anchors

**Files:**
- Modify: `crates/koushi-state/src/state/navigation.rs`
- Modify: `crates/koushi-core/src/store.rs`
- Modify: `crates/koushi-state/src/action.rs`
- Modify: `crates/koushi-state/src/reducer/navigation.rs`
- Modify: `apps/desktop/src/components/TimelineView.tsx`
- Modify: relevant DTO/types/mocks after Rust state shape changes

- [x] **Step 1: Add state tests**

Add tests proving encrypted navigation save/load preserves:

```rust
TimelineScrollAnchor {
    event_id: "$event:example.invalid".to_owned(),
    offset_px: 32,
    updated_at_ms: 1_719_000_000_000,
}
```

Also test legacy navigation JSON without `room_scroll_anchors` loads with an empty map.

- [x] **Step 2: Add bounded model**

Add:

```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelineScrollAnchor {
    pub event_id: String,
    pub offset_px: i32,
    pub updated_at_ms: u64,
}
```

Add `room_scroll_anchors: BTreeMap<String, TimelineScrollAnchor>` to `NavigationState` with `#[serde(default)]`.

- [x] **Step 3: Add reducer action**

Add an `AppAction::TimelineScrollAnchorUpdated { room_id, anchor }` and reducer branch that:

1. ignores updates when session is not ready;
2. inserts the anchor;
3. evicts entries beyond 200 by oldest `updated_at_ms`;
4. emits existing navigation persistence effects.

- [x] **Step 4: Add frontend capture**

In `TimelineView.tsx`, reuse the stable timeline item ids already used for scroll anchoring. Throttle dispatch to avoid more than one anchor update per 1000ms per room. Store offset relative to the top of the anchored message element.

- [x] **Step 5: Add restore behavior**
  Room anchors now restore through the live room timeline actor with bounded backward pagination. When the anchor is already rendered, the existing DOM restore path runs. When it is missing from the live window, the frontend requests a one-shot live restore and the actor continues paging until the anchor enters the normal live `navigation_items` stream or the budget is exhausted. The focused-event timeline bootstrap path remains unused.

- [x] **Step 6: Verify**

Run:

```bash
cargo test -p koushi-state navigation_state
cargo test -p koushi-core navigation
cd apps/desktop && npm test -- TimelineView
cd apps/desktop && npm run typecheck
```

- [x] **Step 7: Commit**

```bash
git add crates/koushi-state crates/koushi-core apps/desktop/src apps/desktop/src-tauri
git commit -m "feat: persist per-room timeline anchors"
```

## Task 4: Room-List Hot Path Without Full Members

**Files:**
- Modify: `crates/koushi-sdk/src/lib.rs`
- Modify: `crates/koushi-state/src/reducer/profile.rs`
- Modify: `crates/koushi-state/tests/profile_state.rs`
- Test: `crates/koushi-sdk/src/lib.rs`

- [x] **Step 1: Write source-guard tests**

Add tests asserting the body of `matrix_room_list_snapshot_from_rooms` does not contain:

```rust
"collect_active_member_profiles("
"room.members(matrix_sdk::RoomMemberships::ACTIVE)"
```

Keep a separate test allowing `matrix_room_member_summaries` to use `room.members(...)` for Room Info.

- [x] **Step 2: Preserve DM behavior tests**

Add tests/source guards for direct account-data helper and DM fallback so one-to-one DMs still resolve counterpart labels and avatar profile fallback.

- [x] **Step 3: Implement hot-path rewrite**

Replace `active_user_ids` usage in `matrix_room_list_snapshot_from_rooms`:

```rust
let joined_members = room.joined_members_count();
let direct_dm_user_ids = direct_targets_by_room.get(&room_id).cloned().unwrap_or_default();
let dm_user_ids = if !direct_dm_user_ids.is_empty() {
    direct_dm_user_ids
} else if is_dm {
    resolve_dm_counterpart_user_ids_lightweight(&room, &own_user_id).await
} else {
    Vec::new()
};
```

Implemented as `matrix_room_list_dm_user_ids`, preserving valid direct/cached/hero DM IDs even when local profile data is absent. Space membership remains best-effort via a space-only `members_no_sync` helper so existing DM-to-space grouping does not regress. `UserProfilesUpdated` now merges partial profile updates instead of replacing the cache.

- [x] **Step 4: Verify**

Run:

```bash
cargo test -p koushi-sdk room_list
cargo test -p koushi-sdk direct_account_data
cargo test -p koushi-sdk dm
cargo test -p koushi-sdk space_member_ids_are_no_sync_and_space_only
cargo test -p koushi-state user_profile_cache_merges_partial_rust_snapshots
cargo test -p koushi-state user_profile_avatar_thumbnail_is_preserved_across_partial_profile_update
cargo test -p koushi-core normalize_rooms_assigns_dm_space_ids_by_counterpart_membership
```

- [x] **Step 5: Commit**

```bash
git add crates/koushi-sdk/src/lib.rs crates/koushi-state/src/reducer/profile.rs crates/koushi-state/tests/profile_state.rs
git commit -m "perf: avoid full member loads in room-list projection"
```

## Task 5: Member-List Virtualization And Visible Avatars

**Files:**
- Modify: `apps/desktop/src/App.tsx`
- Modify: `apps/desktop/src/App.test.tsx`
- Modify: `apps/desktop/src/components/RoomInfoPanel.tsx`
- Modify: `apps/desktop/src/components/RoomInfoPanel.test.tsx`
- Modify: `apps/desktop/src/components/rightPanel.tsx`
- Modify: `apps/desktop/src/styles.css`

- [x] **Step 1: Write component tests**

Added tests proving a 3000-member room renders only a bounded visible row count, reveals late rows after scrolling, exposes a keyboard-focusable member scroll region, and requests avatars only for rendered/overscanned rows. Added an App-level guard proving Room Info member-avatar requests respect the global avatar-thumbnail download gate.

- [x] **Step 2: Implement virtualization**

Implemented a local fixed-height windowing helper with a 92px CSS/TS row-height contract, top/bottom spacers, a bounded member scroll container, local scroll reset on room/member-count changes, and `aria-posinset`/`aria-setsize` metadata for virtual rows:

```ts
const rowHeight = 92;
const overscan = 4;
const start = Math.max(0, Math.floor(scrollTop / rowHeight) - overscan);
const visibleCount = Math.ceil(viewportHeight / rowHeight) + overscan * 2;
const visibleMembers = members.slice(start, start + visibleCount);
```

- [x] **Step 3: Visible avatar requests**

Room Info now requests member avatars only from `visibleMembers`, skips null MXCs, de-duplicates per mounted panel, and removes failed MXCs from the set so a later visible pass can retry. `App` only passes the thumbnail transport when `AVATAR_THUMBNAIL_DOWNLOADS_ENABLED` is enabled.

- [x] **Step 4: Verify**

Run:

```bash
cd apps/desktop && npm test -- RoomInfoPanel
cd apps/desktop && npm test -- App.test.tsx
cd apps/desktop && npm run typecheck
cd apps/desktop && npm test
cd apps/desktop && npm run lint
git diff --check
```

- [x] **Step 5: Commit**

```bash
git add apps/desktop/src/App.tsx apps/desktop/src/App.test.tsx apps/desktop/src/components/RoomInfoPanel.tsx apps/desktop/src/components/RoomInfoPanel.test.tsx apps/desktop/src/components/rightPanel.tsx apps/desktop/src/styles.css docs/superpowers/plans/2026-06-22-issue-118-completion.md docs/superpowers/specs/2026-06-22-issue-118-completion-design.md
git commit -m "perf: virtualize room member list"
```

## Task 6: Renderable Thumbnail Cache Encryption / Plaintext Removal

**Files:**
- Add: `crates/koushi-core/src/renderable_thumbnail.rs`
- Modify: `crates/koushi-core/src/account.rs`
- Modify: `crates/koushi-core/src/link_preview.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src/domain/mediaUrl.ts`
- Modify: focused tests in `crates/koushi-core`, `apps/desktop/src-tauri`, and `apps/desktop/src/domain/mediaUrl.test.ts`

- [x] **Step 1: Write plaintext regression tests**

Add tests that seed `avatar_thumbnails/` and `link_preview_thumbnails/`, run cleanup, and assert no plaintext files remain while `media_downloads/` survives. Add tests that the new renderable thumbnail helper returns `koushi-thumbnail://localhost/<kind>/<key>` rather than `file://`, account/session cleanup clears the in-memory cache, the Tauri protocol handler serves only known in-memory refs, CSP permits `koushi-thumbnail`, and release preflight rejects broad app-data asset scope.

- [x] **Step 2: Implement encrypted render path**

Use a custom-protocol backed in-memory renderable cache:

```rust
pub enum RenderableThumbnailKind {
    Avatar,
    LinkPreview,
}
```

`download_avatar_thumbnail` and `download_preview_image` still fetch through the SDK media layer, but automatic thumbnails must avoid persistent SDK media caching when the SDK API exposes that choice. They store the resulting decrypted bytes only in a bounded process-memory cache and return `koushi-thumbnail://localhost/avatar/<key>` or `koushi-thumbnail://localhost/link-preview/<key>`. The Tauri `koushi-thumbnail` protocol handler resolves only these opaque refs from memory and responds with bytes plus MIME type; it never reads arbitrary files. A cold process may refetch automatic thumbnails through the existing bounded visible-range request path.

The Tauri CSP must allow `koushi-thumbnail:` in `img-src`, and static `assetProtocol.scope` must not expose broad app-data paths. Leave only explicit `media_downloads` in static/runtime asset scope.

- [x] **Step 3: Cleanup old plaintext**

Add a startup cleanup after the encrypted serve path is registered:

```rust
for dir in ["avatar_thumbnails", "link_preview_thumbnails"] {
    let _ = std::fs::remove_dir_all(data_dir.join(dir));
}
```

Do not remove `media_downloads`, because those are explicit user-requested downloads. Remove only `avatar_thumbnails` and `link_preview_thumbnails` from the Tauri asset scope; keep `media_downloads`.

- [x] **Step 4: Verify**

Run:

```bash
cargo test -p koushi-core thumbnail link_preview
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml renderable_asset_cache
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml renderable_thumbnail_protocol
cd apps/desktop && npm test -- mediaUrl
cd apps/desktop && npm test -- TimelineView
cd apps/desktop && npm test -- releaseScripts
cd apps/desktop && npm run typecheck
```

- [x] **Step 5: Commit**

```bash
git add crates/koushi-core apps/desktop/src-tauri apps/desktop/src
git commit -m "fix: remove plaintext renderable thumbnail cache"
```

## Task 7: Search Crawler Pause And Progress UI

**Files:**
- Modify: `crates/koushi-state/src/reducer/settings.rs`
- Modify: `crates/koushi-state/tests/search_crawler_state.rs`
- Modify: `apps/desktop/src/components/UserSettingsPanel.tsx`
- Modify: `apps/desktop/src/components/UserSettingsPanel.test.tsx`
- Modify: `apps/desktop/e2e/search-crawler-settings.spec.ts`

- [x] **Step 1: Write reducer test**

Add a test that starts with a `Running` room, applies settings patch to `SearchCrawlerSpeed::Paused`, and asserts no room remains `Running`.

- [x] **Step 2: Implement pause transition**

In the settings reducer pause branch:

```rust
for room_state in state.search_crawler.rooms.values_mut() {
    if matches!(room_state, SearchCrawlerRoomState::Running { .. }) {
        *room_state = SearchCrawlerRoomState::Queued;
    }
}
effects.push(AppEffect::EmitUiEvent(UiEvent::SearchCrawlerChanged));
```

Use a new `Paused` state only if the UI needs a distinct state.

- [x] **Step 3: Improve summary UI**

Replace raw summary as primary text with:

```ts
const summary =
  crawlerPaused
    ? t("settings.searchHistoryPausedProgress", { completed, total })
    : t("settings.searchHistoryIndexingProgress", { completed, total });
```

Keep raw counts in detail rows.

- [x] **Step 4: Verify existing done items**

Keep tests for active speed `aria-pressed`, separate Room index panel, and scrollable long list.

- [x] **Step 5: Verify**

Run:

```bash
cargo test -p koushi-state search_crawler
cd apps/desktop && npm test -- UserSettingsPanel
cd apps/desktop && npx playwright test e2e/search-crawler-settings.spec.ts
```

- [ ] **Step 6: Commit**

```bash
git add crates/koushi-state apps/desktop/src/components/UserSettingsPanel.tsx apps/desktop/src/components/UserSettingsPanel.test.tsx apps/desktop/e2e/search-crawler-settings.spec.ts apps/desktop/src/i18n/messages.ts
git commit -m "fix: make search crawler pause and progress accurate"
```

## Task 8: QoS Guardrails And Large-Account Regression

**Files:**
- Modify: `crates/koushi-core/src/runtime.rs`
- Modify: `crates/koushi-core/src/account.rs`
- Modify: `crates/koushi-core/src/command.rs`
- Modify: `crates/koushi-core/tests/runtime_intent_lifecycle.rs`
- Modify: `crates/koushi-core/src/bin/headless-core-qa.rs`

- [ ] **Step 1: Add saturated-background test**

Extend the existing #116 runtime/QA harness so background avatar/search/room-list work is saturated, then submit `SelectRoom`. Assert terminal `IntentLifecycle` and first timeline paint under the deterministic 1s budget.

- [ ] **Step 2: Make lane semantics explicit**

Document in code and enforce:

- user intent: reliable, request-id correlated, never dropped silently;
- foreground active-room work: bounded and prioritized;
- background work: coalesced/latest-wins/drop-on-full only when recoverable.

- [ ] **Step 3: Verify**

Run:

```bash
cargo test -p koushi-core runtime_intent_lifecycle
cargo test -p koushi-core search_crawler_room_notifications_are_latest_wins_and_nonblocking
```

- [ ] **Step 4: Commit**

```bash
git add crates/koushi-core
git commit -m "test: guard room selection against background floods"
```

## Task 9: Final Verification And Issue Updates

**Files:**
- Modify: `docs/superpowers/plans/2026-06-22-issue-118-completion.md` checkboxes as tasks complete
- No production code unless verification finds a bug

- [ ] **Step 1: Run full targeted suite**

Run:

```bash
cargo test -p koushi-state
cargo test -p koushi-sdk
cargo test -p koushi-core
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
cd apps/desktop && npm run typecheck
cd apps/desktop && npm test
cd apps/desktop && npx playwright test e2e/search-crawler-settings.spec.ts
```

- [ ] **Step 2: Main-agent review**

Review the full diff from the design commit:

```bash
git diff 9b563f6..HEAD
```

Check every acceptance criterion in `docs/superpowers/specs/2026-06-22-issue-118-completion-design.md`.

- [ ] **Step 3: Claude CLI review**

Run:

```bash
claude -p --permission-mode dontAsk --effort high \
  "Review the implementation since commit 9b563f6 for full compliance with docs/superpowers/specs/2026-06-22-issue-118-completion-design.md. Do not edit files. Return findings only."
```

Fix every P1/P2 finding or document why it is not applicable with code evidence.

- [ ] **Step 4: Update GitHub issues**

Post a private-data-free issue comment on #118 with:

- checklist mapping and evidence;
- test commands and pass/fail;
- #117 status;
- any manual real-account QA result with counts/timings only.

Close #117 only if its plaintext-cache acceptance criteria are met. Close #118 only if every mapped checkbox is complete.

- [ ] **Step 5: Final commit**

```bash
git add docs/superpowers/plans/2026-06-22-issue-118-completion.md
git commit -m "docs: record issue 118 completion evidence"
```
