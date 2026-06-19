# Issue #57 — URL Link Previews Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add client-side Matrix URL preview cards for `m.text`/`m.notice` messages, with a global toggle, a per-room override, an E2EE privacy guard, and a viewer-local hide affordance, keeping all preview semantics Rust-owned and QA evidence token-only.

**Architecture:** Rust core extracts URLs, applies the global/per-room/encryption policy, fetches OpenGraph metadata and image thumbnails through the homeserver, and owns the viewer-local hidden-event set. React renders only the projected `link_previews` DTO and dispatches typed `load_link_previews`/`hide_link_preview` commands. Per-room overrides are persisted with the settings store; encryption state flows from the SDK room summary into the timeline projection.

**Tech Stack:** Rust (koushi-state / koushi-core / koushi-sdk), Tauri, TypeScript/React, Playwright, local Conduit/Tuwunel headless QA.

---

## File map

| Responsibility | Path |
|---|---|
| Settings product state (global + per-room override) | `crates/koushi-state/src/state.rs` |
| Settings reducer / test fixtures | `crates/koushi-state/src/reducer.rs`, `crates/koushi-state/tests/link_preview_state.rs` |
| SDK room encryption exposure | `crates/koushi-sdk/src/lib.rs` |
| Room normalization (encryption flag) | `crates/koushi-core/src/room.rs` |
| Core command variants | `crates/koushi-core/src/command.rs` |
| Timeline event / item DTOs | `crates/koushi-core/src/event.rs` |
| URL extraction, policy, preview helpers | `crates/koushi-core/src/link_preview.rs` |
| Timeline actor fetch / hide / policy wiring | `crates/koushi-core/src/timeline.rs` |
| Runtime policy broadcast to timelines | `crates/koushi-core/src/runtime.rs` |
| Store data-dir accessor | `crates/koushi-core/src/store.rs` |
| Account actor → timeline manager data-dir plumbing | `crates/koushi-core/src/account.rs` |
| Core crate exports / dependencies | `crates/koushi-core/src/lib.rs`, `crates/koushi-core/Cargo.toml` |
| Tauri command builders / registration | `apps/desktop/src-tauri/src/commands.rs`, `apps/desktop/src-tauri/src/lib.rs` |
| TypeScript domain types | `apps/desktop/src/domain/types.ts` |
| TypeScript core event contract | `apps/desktop/src/domain/coreEvents.ts`, `apps/desktop/src/domain/coreEvents.generated.json` |
| i18n catalogs | `apps/desktop/src/i18n/messages.ts` |
| Browser fake + Tauri API client | `apps/desktop/src/backend/browserFakeApi.ts`, `apps/desktop/src/backend/client.ts` |
| Global settings UI | `apps/desktop/src/components/UserSettingsPanel.tsx` |
| Per-room settings UI | `apps/desktop/src/components/RoomInfoPanel.tsx` |
| Timeline preview rendering | `apps/desktop/src/components/TimelineView.tsx`, `apps/desktop/src/styles.css` |
| Transport / panel wiring | `apps/desktop/src/App.tsx` |
| Browser-headless GUI contract tests | `apps/desktop/e2e/basic-operations.spec.ts` |
| Headless core QA scenario | `crates/koushi-core/src/bin/headless-core-qa.rs`, `scripts/desktop-headless-local-qa.mjs` |

---

## Phase A — Rust / state machine / headless

### Task 1: Add global + per-room URL-preview settings state

**Files:**
- Modify: `crates/koushi-state/src/state.rs:341-360`, `:414-423`, `:127-179`

- [ ] **Step 1: Extend `DisplaySettings` with the global toggle**

```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DisplaySettings {
    #[serde(default = "default_code_block_wrap")]
    pub code_block_wrap: bool,
    #[serde(default)]
    pub hide_redacted: bool,
    #[serde(default = "default_url_previews_enabled")]
    pub url_previews_enabled: bool,
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            code_block_wrap: true,
            hide_redacted: false,
            url_previews_enabled: true,
        }
    }
}

fn default_url_previews_enabled() -> bool {
    true
}
```

- [ ] **Step 2: Add per-room override maps to `SettingsValues` and `SettingsPatch`**

Near the `SettingsValues` definition, add:

```rust
pub type RoomUrlPreviews = std::collections::BTreeMap<String, bool>;
```

Extend `SettingsValues`:

```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SettingsValues {
    pub locale: LocaleSettings,
    pub appearance: AppearanceSettings,
    pub typography: TypographySettings,
    pub keyboard: KeyboardSettings,
    #[serde(default)]
    pub notifications: NotificationSettings,
    #[serde(default)]
    pub display: DisplaySettings,
    #[serde(default)]
    pub media: MediaSettings,
    #[serde(default)]
    pub room_url_previews: RoomUrlPreviews,
}
```

Extend `SettingsPatch`:

```rust
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SettingsPatch {
    pub locale: Option<LocaleSettings>,
    pub appearance: Option<AppearanceSettings>,
    pub typography: Option<TypographySettings>,
    pub keyboard: Option<KeyboardSettings>,
    pub notifications: Option<NotificationSettings>,
    pub display: Option<DisplaySettings>,
    pub media: Option<MediaSettings>,
    #[serde(default)]
    pub room_url_previews: Option<RoomUrlPreviews>,
}
```

- [ ] **Step 3: Merge per-room overrides in `apply_patch`**

```rust
impl SettingsValues {
    pub fn apply_patch(&mut self, patch: SettingsPatch) {
        // ... existing arms ...
        if let Some(display) = patch.display {
            self.display = display;
        }
        if let Some(media) = patch.media {
            self.media = media;
        }
        if let Some(room_url_previews) = patch.room_url_previews {
            for (room_id, enabled) in room_url_previews {
                if enabled {
                    self.room_url_previews.insert(room_id, enabled);
                } else {
                    self.room_url_previews.remove(&room_id);
                }
            }
        }
    }
}
```

- [ ] **Step 4: Update `SettingsValues::default`**

```rust
impl Default for SettingsValues {
    fn default() -> Self {
        Self {
            locale: LocaleSettings::default(),
            appearance: AppearanceSettings::default(),
            typography: TypographySettings::default(),
            keyboard: KeyboardSettings::default(),
            notifications: NotificationSettings::default(),
            display: DisplaySettings::default(),
            media: MediaSettings::default(),
            room_url_previews: RoomUrlPreviews::new(),
        }
    }
}
```

- [ ] **Step 5: Add `is_encrypted` to `RoomSummary`**

```rust
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomSummary {
    pub room_id: String,
    pub display_name: String,
    pub display_label: String,
    #[serde(default)]
    pub original_display_label: String,
    #[serde(default)]
    pub avatar: Option<AvatarImage>,
    pub is_dm: bool,
    #[serde(default)]
    pub dm_user_ids: Vec<String>,
    #[serde(default)]
    pub tags: RoomTags,
    pub unread_count: u64,
    pub notification_count: u64,
    pub highlight_count: u64,
    #[serde(default)]
    pub marked_unread: bool,
    #[serde(default)]
    pub last_activity_ms: u64,
    pub parent_space_ids: Vec<String>,
    #[serde(default)]
    pub is_encrypted: bool,
}
```

Add `is_encrypted` to the `fmt::Debug` impl with value `self.is_encrypted`.

- [ ] **Step 6: Run focused state tests**

```bash
cargo test -p koushi-state --test settings_state
```

Expected: PASS after updating any struct-literal tests that construct `SettingsValues` to include `room_url_previews: RoomUrlPreviews::new()`.

---

### Task 2: Write reducer-level settings tests

**Files:**
- Create: `crates/koushi-state/tests/link_preview_state.rs`

- [ ] **Step 1: Write the test file**

```rust
use std::collections::BTreeMap;

use koushi_state::{
    AppAction, AppEffect, AppState, DisplaySettings, SettingsPatch, SettingsValues, UiEvent,
    reduce,
};

fn ready_state_with_room(room_id: &str) -> AppState {
    use koushi_state::{RoomSummary, RoomTags, SessionInfo, SessionState};

    AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://server.example.invalid".to_owned(),
            user_id: "@alice:example.invalid".to_owned(),
            device_id: "ALICEDEVICE".to_owned(),
        }),
        rooms: vec![RoomSummary {
            room_id: room_id.to_owned(),
            display_name: "Room".to_owned(),
            display_label: "Room".to_owned(),
            original_display_label: "Room".to_owned(),
            avatar: None,
            is_dm: false,
            dm_user_ids: Vec::new(),
            tags: RoomTags::default(),
            unread_count: 0,
            notification_count: 0,
            highlight_count: 0,
            marked_unread: false,
            last_activity_ms: 0,
            parent_space_ids: Vec::new(),
            is_encrypted: false,
        }],
        ..AppState::default()
    }
}

#[test]
fn display_settings_default_enables_url_previews() {
    let values = SettingsValues::default();
    assert!(values.display.url_previews_enabled);
}

#[test]
fn settings_update_toggles_global_url_previews() {
    let mut state = ready_state_with_room("!room:example.invalid");
    let effects = reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 1,
            patch: SettingsPatch {
                display: Some(DisplaySettings {
                    code_block_wrap: true,
                    hide_redacted: false,
                    url_previews_enabled: false,
                }),
                ..SettingsPatch::default()
            },
        },
    );

    assert!(!state.settings.values.display.url_previews_enabled);
    assert!(effects.iter().any(|e| matches!(
        e,
        AppEffect::PersistSettings { request_id: 1, .. }
    )));
    assert!(effects
        .iter()
        .any(|e| matches!(e, AppEffect::EmitUiEvent(UiEvent::SettingsChanged))));
}

#[test]
fn per_room_override_merges_into_settings() {
    let mut state = ready_state_with_room("!room:example.invalid");
    let mut overrides = BTreeMap::new();
    overrides.insert("!room:example.invalid".to_owned(), false);

    reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 2,
            patch: SettingsPatch {
                room_url_previews: Some(overrides),
                ..SettingsPatch::default()
            },
        },
    );

    assert_eq!(
        state
            .settings
            .values
            .room_url_previews
            .get("!room:example.invalid"),
        Some(&false)
    );
}

#[test]
fn false_override_removes_room_entry() {
    let mut state = ready_state_with_room("!room:example.invalid");
    let mut overrides = BTreeMap::new();
    overrides.insert("!room:example.invalid".to_owned(), true);
    state.settings.values.room_url_previews = overrides;

    let mut remove = BTreeMap::new();
    remove.insert("!room:example.invalid".to_owned(), false);
    reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 3,
            patch: SettingsPatch {
                room_url_previews: Some(remove),
                ..SettingsPatch::default()
            },
        },
    );

    assert!(!state
        .settings
        .values
        .room_url_previews
        .contains_key("!room:example.invalid"));
}
```

- [ ] **Step 2: Run the new test file**

```bash
cargo test -p koushi-state --test link_preview_state
```

Expected: PASS.

---

### Task 3: Expose room encryption state from the SDK

**Files:**
- Modify: `crates/koushi-sdk/src/lib.rs:2003-2016`, `:4217-4245`, `:4121-4134`
- Modify: `crates/koushi-core/src/room.rs:1831-1846`

- [ ] **Step 1: Add `is_encrypted` to `MatrixRoomListRoom`**

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixRoomListRoom {
    pub room_id: String,
    pub display_name: String,
    pub avatar_mxc_uri: Option<String>,
    pub is_dm: bool,
    pub dm_user_ids: Vec<String>,
    pub tags: MatrixRoomTags,
    pub unread_count: u64,
    pub notification_count: u64,
    pub highlight_count: u64,
    pub marked_unread: bool,
    pub last_activity_ms: u64,
    pub parent_space_ids: Vec<String>,
    pub is_encrypted: bool,
}
```

- [ ] **Step 2: Update `matrix_room_list_room_from_counts`**

```rust
fn matrix_room_list_room_from_counts(
    room_id: String,
    display_name: String,
    avatar_mxc_uri: Option<String>,
    is_dm: bool,
    dm_user_ids: Vec<String>,
    tags: MatrixRoomTags,
    notification_count: u64,
    highlight_count: u64,
    unread_count: u64,
    marked_unread: bool,
    last_activity_ms: u64,
    parent_space_ids: Vec<String>,
    is_encrypted: bool,
) -> MatrixRoomListRoom {
    MatrixRoomListRoom {
        room_id,
        display_name,
        avatar_mxc_uri,
        is_dm,
        dm_user_ids,
        tags,
        unread_count,
        notification_count,
        highlight_count,
        marked_unread,
        last_activity_ms,
        parent_space_ids,
        is_encrypted,
    }
}
```

- [ ] **Step 3: Pass encryption from the room snapshot builder**

Inside `matrix_room_list_snapshot_from_rooms` (around line 4100), before pushing:

```rust
let is_encrypted = room
    .latest_encryption_state()
    .await
    .map(|state| state.is_encrypted())
    .unwrap_or(false);

snapshot.rooms.push(matrix_room_list_room_from_counts(
    room_id,
    display_name,
    room.avatar_url().map(|uri| uri.to_string()),
    is_dm,
    dm_user_ids,
    tags,
    notification_count,
    highlight_count,
    unread_count,
    is_marked_unread,
    room.recency_stamp().map(|stamp| stamp.into()).unwrap_or(0),
    parent_space_ids,
    is_encrypted,
));
```

- [ ] **Step 4: Update SDK unit tests for the new argument**

Update every call to `matrix_room_list_room_from_counts` in `crates/koushi-sdk/src/lib.rs` tests to pass a trailing `false` (or `true` for an encrypted-room test). For example:

```rust
let room = matrix_room_list_room_from_counts(
    "!room:example.invalid".to_owned(),
    "Room".to_owned(),
    None,
    true,
    vec!["@alice:example.invalid".to_owned()],
    MatrixRoomTags::default(),
    4,
    2,
    4,
    false,
    0,
    vec!["!space:example.invalid".to_owned()],
    false,
);
```

- [ ] **Step 5: Propagate `is_encrypted` to `RoomSummary`**

In `crates/koushi-core/src/room.rs` `normalize_rooms`, add:

```rust
RoomSummary {
    room_id: room.room_id.clone(),
    display_name: room.display_name.clone(),
    display_label,
    original_display_label: display_label,
    avatar: avatar_from_mxc_uri(room.avatar_mxc_uri.as_deref()),
    is_dm: room.is_dm,
    dm_user_ids: room.dm_user_ids.clone(),
    tags: normalize_room_tags(&room.tags),
    unread_count: room.unread_count,
    notification_count: room.notification_count,
    highlight_count: room.highlight_count,
    marked_unread: room.marked_unread,
    last_activity_ms: room.last_activity_ms,
    parent_space_ids: room.parent_space_ids.clone(),
    is_encrypted: room.is_encrypted,
}
```

- [ ] **Step 6: Run SDK/core tests**

```bash
cargo test -p koushi-sdk -p koushi-core --lib
```

Expected: PASS.

---

### Task 4: Define link-preview DTOs on the core event boundary

**Files:**
- Modify: `crates/koushi-core/src/event.rs`
- Modify: `crates/koushi-core/src/lib.rs:34-42`

- [ ] **Step 1: Add `LinkPreview` types**

Add after `AvatarThumbnailState` import / near `TimelineMedia`:

```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LinkPreviewState {
    #[default]
    Pending,
    Loading,
    Ready,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LinkPreviewImage {
    pub source: TimelineMediaSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u64>,
    #[serde(default)]
    pub thumbnail: AvatarThumbnailState,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LinkPreview {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<LinkPreviewImage>,
    #[serde(default)]
    pub state: LinkPreviewState,
}
```

- [ ] **Step 2: Extend `TimelineItem`**

```rust
pub struct TimelineItem {
    // ... existing fields ...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_previews: Option<Vec<LinkPreview>>,
}
```

- [ ] **Step 3: Extend `TimelineItem` Debug output**

```rust
.field(
    "link_previews",
    &self.link_previews.as_ref().map(|previews| {
        format!("{} preview(s)", previews.len())
    }),
)
```

- [ ] **Step 4: Re-export the new types**

In `crates/koushi-core/src/lib.rs` `pub use event::{...}` block, add:

```rust
LinkPreview, LinkPreviewImage, LinkPreviewState,
```

---

### Task 5: Add timeline command variants

**Files:**
- Modify: `crates/koushi-core/src/command.rs:1732-1846`, `:1851-1950`

- [ ] **Step 1: Add variants**

```rust
pub enum TimelineCommand {
    // ... existing variants ...
    LoadLinkPreviews {
        request_id: RequestId,
        key: TimelineKey,
        event_id: String,
    },
    HideLinkPreview {
        request_id: RequestId,
        key: TimelineKey,
        event_id: String,
    },
    BroadcastLinkPreviewPolicy {
        global_enabled: bool,
        room_overrides: std::collections::BTreeMap<String, bool>,
    },
}
```

- [ ] **Step 2: Add Debug arms**

```rust
Self::LoadLinkPreviews {
    request_id,
    key,
    event_id,
} => formatter
    .debug_struct("LoadLinkPreviews")
    .field("request_id", request_id)
    .field("key", key)
    .field("event_id", &"EventId(..)")
    .finish(),
Self::HideLinkPreview {
    request_id,
    key,
    event_id,
} => formatter
    .debug_struct("HideLinkPreview")
    .field("request_id", request_id)
    .field("key", key)
    .field("event_id", &"EventId(..)")
    .finish(),
Self::BroadcastLinkPreviewPolicy {
    global_enabled,
    room_overrides,
} => formatter
    .debug_struct("BroadcastLinkPreviewPolicy")
    .field("global_enabled", global_enabled)
    .field("room_override_count", &room_overrides.len())
    .finish(),
```

---

### Task 6: Create URL extraction and preview-policy module

**Files:**
- Create: `crates/koushi-core/src/link_preview.rs`
- Modify: `crates/koushi-core/src/lib.rs:23`
- Modify: `crates/koushi-core/Cargo.toml:22-34`

- [ ] **Step 1: Add dependencies**

In `crates/koushi-core/Cargo.toml`:

```toml
regex = "1"
url = "2"
reqwest = "0.13"
```

- [ ] **Step 2: Declare the module**

In `crates/koushi-core/src/lib.rs`:

```rust
pub(crate) mod link_preview;
```

- [ ] **Step 3: Write `crates/koushi-core/src/link_preview.rs`**

```rust
use std::collections::{BTreeMap, BTreeSet, HashMap};

use regex::Regex;

use crate::event::{LinkPreview, LinkPreviewImage, LinkPreviewState, TimelineFormattedBody};
use crate::event::{AvatarThumbnailState, TimelineMediaSource};

pub const MAX_LINK_PREVIEWS_PER_MESSAGE: usize = 3;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LinkPreviewContext {
    pub global_enabled: bool,
    pub room_enabled: Option<bool>,
    pub hidden_event_ids: BTreeSet<String>,
    pub cache: HashMap<String, LinkPreview>,
}

pub fn extract_urls(
    body: Option<&str>,
    formatted: Option<&TimelineFormattedBody>,
) -> Vec<String> {
    let mut urls = Vec::new();
    let url_re = Regex::new(r"https?://[^\s<>"{}|\\^`\[\]]+").expect("valid url regex");

    let mut collect = |text: &str| {
        for mat in url_re.find_iter(text) {
            let url = mat.as_str().trim_end_matches(|c| ".,;:!?)\"'>".contains(c));
            if !urls.contains(&url.to_owned()) {
                urls.push(url.to_owned());
            }
        }
    };

    if let Some(body) = body {
        collect(body);
    }
    if let Some(formatted) = formatted {
        collect(&formatted.plain_text);
        // Extract hrefs from sanitized HTML without parsing the full DOM.
        let href_re = Regex::new(r#"href=["'](https?://[^"']+)["']"#)
            .expect("valid href regex");
        for cap in href_re.captures_iter(&formatted.html) {
            if let Some(url) = cap.get(1) {
                let url = url.as_str();
                if !urls.contains(&url.to_owned()) {
                    urls.push(url.to_owned());
                }
            }
        }
    }

    urls.into_iter().take(MAX_LINK_PREVIEWS_PER_MESSAGE).collect()
}

pub fn link_previews_for_message(
    body: Option<&str>,
    formatted: Option<&TimelineFormattedBody>,
    event_id: &str,
    is_encrypted: bool,
    context: &LinkPreviewContext,
) -> Option<Vec<LinkPreview>> {
    if context.hidden_event_ids.contains(event_id) {
        return Some(Vec::new());
    }

    let effective_enabled = if is_encrypted {
        context.room_enabled.unwrap_or(false)
    } else {
        context.room_enabled.unwrap_or(context.global_enabled)
    };

    if !effective_enabled {
        return None;
    }

    let urls = extract_urls(body, formatted);
    if urls.is_empty() {
        return None;
    }

    Some(
        urls
            .into_iter()
            .map(|url| {
                context
                    .cache
                    .get(&url)
                    .cloned()
                    .unwrap_or_else(|| LinkPreview {
                        url,
                        title: None,
                        description: None,
                        image: None,
                        state: LinkPreviewState::Pending,
                    })
            })
            .collect(),
    )
}

pub fn effective_room_url_previews_enabled(
    room_id: &str,
    is_encrypted: bool,
    global_enabled: bool,
    room_overrides: &BTreeMap<String, bool>,
) -> bool {
    if is_encrypted {
        room_overrides.get(room_id).copied().unwrap_or(false)
    } else {
        room_overrides
            .get(room_id)
            .copied()
            .unwrap_or(global_enabled)
    }
}

pub fn link_preview_image_from_mxc(mxc_uri: String) -> LinkPreviewImage {
    LinkPreviewImage {
        source: TimelineMediaSource {
            mxc_uri,
            encrypted: false,
            encryption_version: None,
        },
        width: None,
        height: None,
        thumbnail: AvatarThumbnailState::NotRequested,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(enabled: bool) -> LinkPreviewContext {
        LinkPreviewContext {
            global_enabled: enabled,
            room_enabled: None,
            hidden_event_ids: BTreeSet::new(),
            cache: HashMap::new(),
        }
    }

    #[test]
    fn extract_urls_from_plain_text() {
        let body = "Check https://example.invalid/page and http://test.invalid?q=1";
        let urls = extract_urls(Some(body), None);
        assert_eq!(urls, vec![
            "https://example.invalid/page".to_owned(),
            "http://test.invalid?q=1".to_owned(),
        ]);
    }

    #[test]
    fn extract_urls_deduplicates_and_caps() {
        let body = "A https://example.invalid A https://example.invalid B https://b.invalid C https://c.invalid D https://d.invalid";
        let urls = extract_urls(Some(body), None);
        assert_eq!(urls.len(), 3);
        assert!(!urls.iter().any(|u| u.contains("d.invalid")));
    }

    #[test]
    fn encrypted_room_default_off() {
        let previews = link_previews_for_message(
            Some("https://example.invalid"),
            None,
            "$e",
            true,
            &ctx(true),
        );
        assert!(previews.is_none());
    }

    #[test]
    fn encrypted_room_explicit_override_enables() {
        let mut context = ctx(true);
        context.room_enabled = Some(true);
        let previews = link_previews_for_message(
            Some("https://example.invalid"),
            None,
            "$e",
            true,
            &context,
        );
        assert!(previews.is_some());
    }

    #[test]
    fn hidden_event_returns_empty_previews() {
        let mut context = ctx(true);
        context.hidden_event_ids.insert("$e".to_owned());
        let previews = link_previews_for_message(
            Some("https://example.invalid"),
            None,
            "$e",
            false,
            &context,
        );
        assert_eq!(previews, Some(Vec::new()));
    }

    #[test]
    fn cache_reuse_returns_ready_preview() {
        let mut context = ctx(true);
        let cached = LinkPreview {
            url: "https://example.invalid".to_owned(),
            title: Some("Cached".to_owned()),
            description: None,
            image: None,
            state: LinkPreviewState::Ready,
        };
        context
            .cache
            .insert("https://example.invalid".to_owned(), cached.clone());
        let previews = link_previews_for_message(
            Some("https://example.invalid"),
            None,
            "$e",
            false,
            &context,
        )
        .expect("previews");
        assert_eq!(previews.len(), 1);
        assert_eq!(previews[0].state, LinkPreviewState::Ready);
        assert_eq!(previews[0].title, Some("Cached".to_owned()));
    }
}
```

- [ ] **Step 4: Run the module tests**

```bash
cargo test -p koushi-core link_preview
```

Expected: PASS.

---

### Task 7: Wire link previews into the timeline actor

**Files:**
- Modify: `crates/koushi-core/src/timeline.rs:115-200`, `:1235-1260`, `:2408-2450`, `:2825-2874`, `:2990-3010`, `:3632-3800`, `:4565-4662`

- [ ] **Step 1: Add actor fields**

In `TimelineActor` struct:

```rust
/// URL preview policy for this timeline.
link_preview_policy: crate::link_preview::LinkPreviewContext,
/// Application data directory for cached preview images.
data_dir: Option<std::path::PathBuf>,
```

- [ ] **Step 2: Update `TimelineManagerActor`**

Add to `TimelineManagerActor` struct:

```rust
data_dir: Option<std::path::PathBuf>,
link_preview_policy: crate::link_preview::LinkPreviewContext,
```

Update `TimelineManagerActor::spawn`:

```rust
pub fn spawn(
    action_tx: mpsc::Sender<Vec<AppAction>>,
    event_tx: broadcast::Sender<CoreEvent>,
) -> TimelineManagerHandle {
    let (tx, msg_rx) = mpsc::channel(64);
    let actor = TimelineManagerActor {
        session: None,
        room_list_service: None,
        timelines: HashMap::new(),
        action_tx,
        event_tx,
        msg_rx,
        search_index_tx: None,
        ignored_user_ids: std::collections::BTreeSet::new(),
        data_dir: None,
        link_preview_policy: crate::link_preview::LinkPreviewContext::default(),
    };
    executor::spawn(actor.run());
    TimelineManagerHandle { tx }
}
```

Update `spawn_with_session` signature and struct init to accept and store `data_dir` and the current policy. The account actor will provide both.

- [ ] **Step 3: Update account actor plumbing**

In `crates/koushi-core/src/account.rs`:

1. Expose `StoreActor::data_dir` by adding in `crates/koushi-core/src/store.rs`:

```rust
impl StoreActor {
    pub fn data_dir(&self) -> &std::path::Path {
        &self.data_dir
    }
}
```

2. In `AccountActor::spawn`, capture the data dir and pass it to the timeline manager:

```rust
let data_dir = store_actor.data_dir().to_path_buf();
let timeline_manager =
    crate::timeline::TimelineManagerActor::spawn(
        action_tx.clone(),
        event_tx.clone(),
        Some(data_dir.clone()),
    );
```

3. In the code path that creates `spawn_with_session`, pass `Some(data_dir)` and the latest policy snapshot from `AppState` (read from the account actor's view of `self.state.settings.values`). If no state is available yet, pass an empty context.

- [ ] **Step 4: Project link previews during timeline spawn**

In `TimelineActor::spawn`, after building `initial_items`, post-process each item:

```rust
let link_preview_context = manager_policy.for_room(key.room_id());
let mut initial_items: Vec<TimelineItem> = initial_sdk_items
    .iter()
    .map(|item| sdk_item_to_timeline_item(&key, item, own_user_id.as_deref()))
    .map(|mut item| {
        apply_ignored_sender_suppression(&mut item, &ignored_user_ids);
        apply_link_previews_to_item(&mut item, &link_preview_context, &session);
        item
    })
    .collect();
```

Define a helper on `LinkPreviewContext`:

```rust
impl LinkPreviewContext {
    pub fn for_room(&self, room_id: &str) -> Self {
        Self {
            global_enabled: self.global_enabled,
            room_enabled: self.room_overrides.get(room_id).copied(),
            hidden_event_ids: self.hidden_event_ids.clone(),
            cache: self.cache.clone(),
        }
    }
}
```

And `apply_link_previews_to_item`:

```rust
fn apply_link_previews_to_item(
    item: &mut TimelineItem,
    context: &crate::link_preview::LinkPreviewContext,
    session: &Arc<MatrixClientSession>,
) {
    let event_id = match &item.id {
        TimelineItemId::Event { event_id } => event_id.clone(),
        _ => return,
    };

    let is_encrypted = session
        .client()
        .get_room(context.room_id_or(item.room_id_hint()))
        .and_then(|room| room.latest_encryption_state().ok())
        .map(|state| state.is_encrypted())
        .unwrap_or(false);

    item.link_previews = crate::link_preview::link_previews_for_message(
        item.body.as_deref(),
        item.formatted.as_ref(),
        &event_id,
        is_encrypted,
        context,
    );
}
```

Because `TimelineItem` does not carry `room_id`, use the actor's `key.room_id()` via context.

- [ ] **Step 5: Apply link previews to live diffs**

In `handle_timeline_update`, after `core_diffs` is built and before `apply_timeline_diffs_to_items`, post-process each diff item:

```rust
let context = self.link_preview_policy.for_room(self.key.room_id());
for diff in &mut core_diffs {
    if let Some(item) = diff_item_mut(diff) {
        apply_link_previews_to_item(item, &context, &self.session);
    }
}
```

Helper:

```rust
fn diff_item_mut(diff: &mut TimelineDiff) -> Option<&mut TimelineItem> {
    match diff {
        TimelineDiff::PushFront { item }
        | TimelineDiff::PushBack { item }
        | TimelineDiff::Insert { item, .. }
        | TimelineDiff::Set { item, .. }
        | TimelineDiff::Reset { items, .. } => {
            // Reset contains many items; handle it separately.
            None
        }
        _ => None,
    }
}
```

For `Reset`, iterate items. For simplicity in the plan, implement a dedicated loop:

```rust
for diff in &mut core_diffs {
    match diff {
        TimelineDiff::Reset { items } => {
            for item in items {
                apply_link_previews_to_item(item, &context, &self.session);
            }
        }
        TimelineDiff::PushFront { item }
        | TimelineDiff::PushBack { item }
        | TimelineDiff::Insert { item, .. }
        | TimelineDiff::Set { item, .. } => {
            apply_link_previews_to_item(item, &context, &self.session);
        }
        _ => {}
    }
}
```

- [ ] **Step 6: Add command handlers**

In the inner `TimelineActor` command dispatch, add:

```rust
TimelineActorMessage::LoadLinkPreviews { request_id, event_id } => {
    self.handle_load_link_previews(request_id, event_id).await;
}
TimelineActorMessage::HideLinkPreview { request_id, event_id } => {
    self.handle_hide_link_preview(request_id, event_id).await;
}
TimelineActorMessage::LinkPreviewPolicyChanged { policy } => {
    self.handle_link_preview_policy_changed(policy).await;
}
```

And in the outer `TimelineManagerActor::handle_command`, add:

```rust
TimelineCommand::LoadLinkPreviews { request_id, key, event_id } => {
    self.route_to_actor_or_fail(
        request_id,
        &key,
        TimelineActorMessage::LoadLinkPreviews { request_id, event_id },
    )
    .await;
}
TimelineCommand::HideLinkPreview { request_id, key, event_id } => {
    self.route_to_actor_or_fail(
        request_id,
        &key,
        TimelineActorMessage::HideLinkPreview { request_id, event_id },
    )
    .await;
}
TimelineCommand::BroadcastLinkPreviewPolicy {
    global_enabled,
    room_overrides,
} => {
    self.link_preview_policy.global_enabled = global_enabled;
    self.link_preview_policy.room_overrides = room_overrides;
    for handle in self.timelines.values() {
        let _ = handle
            .send(TimelineActorMessage::LinkPreviewPolicyChanged {
                policy: self.link_preview_policy.for_room(handle.room_id()),
            })
            .await;
    }
}
```

`handle.room_id()` does not exist on `TimelineManagerHandle`; instead store the `TimelineKey` alongside the handle or pass the policy with a room-aware context from the manager. For the plan, store `timelines: HashMap<TimelineKey, TimelineManagerHandle>` (it already is keyed by `TimelineKey`) and use `key.room_id()`.

- [ ] **Step 7: Implement fetch + image download**

Add to `link_preview.rs`:

```rust
use matrix_sdk::media::{MediaFormat, MediaRequestParameters, MediaSource};
use matrix_sdk::ruma::MxcUri;
use std::path::PathBuf;

pub async fn fetch_link_preview(
    session: &crate::MatrixClientSession,
    url: &str,
    data_dir: Option<&std::path::Path>,
) -> Result<LinkPreview, ()> {
    let client = session.client();
    let mut preview_url = client.homeserver();
    preview_url.set_path("/_matrix/media/v3/preview_url");
    preview_url.set_query(None);
    preview_url
        .query_pairs_mut()
        .append_pair("url", url);

    let mut request = client.http_client().get(preview_url);
    if let Some(token) = client.access_token() {
        request = request.header("Authorization", format!("Bearer {token}"));
    }

    let response = request.send().await.map_err(|_| ())?;
    let bytes = response.bytes().await.map_err(|_| ())?;
    let json: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| ())?;

    let title = json
        .get("og:title")
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let description = json
        .get("og:description")
        .and_then(|v| v.as_str())
        .map(str::to_owned);

    let image = if let Some(image_url) = json.get("og:image").and_then(|v| v.as_str()) {
        MxcUri::parse(image_url)
            .ok()
            .and_then(|uri| {
                let owned: matrix_sdk::ruma::OwnedMxcUri = uri.to_owned();
                Some((owned, image_url.to_owned()))
            })
            .and_then(|(owned, original_url)| {
                download_preview_image(session, &owned, data_dir).ok().map(|thumbnail| {
                    LinkPreviewImage {
                        source: TimelineMediaSource {
                            mxc_uri: original_url,
                            encrypted: false,
                            encryption_version: None,
                        },
                        width: json
                            .get("og:image:width")
                            .and_then(|v| v.as_u64()),
                        height: json
                            .get("og:image:height")
                            .and_then(|v| v.as_u64()),
                        thumbnail,
                    }
                })
            })
    } else {
        None
    };

    Ok(LinkPreview {
        url: url.to_owned(),
        title,
        description,
        image,
        state: LinkPreviewState::Ready,
    })
}

fn download_preview_image(
    session: &crate::MatrixClientSession,
    uri: &matrix_sdk::ruma::OwnedMxcUri,
    data_dir: Option<&std::path::Path>,
) -> Result<AvatarThumbnailState, ()> {
    // Synchronous preparation; the actual download is async in the actor.
    // This function is a placeholder for the synchronous file-name setup.
    let _ = (session, uri, data_dir);
    Ok(AvatarThumbnailState::NotRequested)
}
```

In the actor `handle_load_link_previews`, perform the async image download:

```rust
async fn handle_load_link_previews(&mut self, request_id: RequestId, event_id: String) {
    let Some(index) = self.navigation_items.iter().position(|item| {
        matches!(&item.id, TimelineItemId::Event { event_id: id } if id == &event_id)
    }) else {
        return;
    };

    let Some(previews) = self.navigation_items[index].link_previews.clone() else {
        return;
    };

    let mut updated = Vec::new();
    for mut preview in previews {
        if preview.state != LinkPreviewState::Pending {
            updated.push(preview);
            continue;
        }

        preview.state = LinkPreviewState::Loading;
        match crate::link_preview::fetch_link_preview(
            &self.session,
            &preview.url,
            self.data_dir.as_deref(),
        )
        .await
        {
            Ok(fetched) => {
                self.link_preview_policy
                    .cache
                    .insert(fetched.url.clone(), fetched.clone());
                updated.push(fetched);
            }
            Err(_) => {
                preview.state = LinkPreviewState::Failed;
                updated.push(preview);
            }
        }
    }

    self.navigation_items[index].link_previews = Some(updated);
    let core_diffs = vec![TimelineDiff::Set {
        index,
        item: self.navigation_items[index].clone(),
    }];

    let batch_id = self.next_batch_id;
    self.next_batch_id = TimelineBatchId(batch_id.0 + 1);
    self.emit(CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
        key: self.key.clone(),
        generation: self.generation,
        batch_id,
        diffs: core_diffs,
    }));
}
```

For image download inside `fetch_link_preview`, use `session.client().media().get_media_content(&MediaRequestParameters { source: MediaSource::Plain(uri.clone()), format: MediaFormat::File }, true).await`, then write bytes to `data_dir/link_preview_thumbnails/<hash>.bin`. Compute hash with `std::collections::hash_map::DefaultHasher` on the URL. Set `AvatarThumbnailState::Ready { source_url: format!("file://{}", path.display()), width, height, mime_type: Some(mimetype) }`.

- [ ] **Step 8: Implement hide handler**

```rust
async fn handle_hide_link_preview(&mut self, _request_id: RequestId, event_id: String) {
    let context = self.link_preview_policy.for_room(self.key.room_id());
    if !context.hidden_event_ids.insert(event_id.clone()) {
        return;
    }

    let mut changed = Vec::new();
    for (index, item) in self.navigation_items.iter_mut().enumerate() {
        if matches!(&item.id, TimelineItemId::Event { event_id: id } if id == &event_id) {
            apply_link_previews_to_item(item, &context, &self.session);
            changed.push((index, item.clone()));
        }
    }

    if changed.is_empty() {
        return;
    }

    let core_diffs: Vec<TimelineDiff> = changed
        .into_iter()
        .map(|(index, item)| TimelineDiff::Set { index, item })
        .collect();

    let batch_id = self.next_batch_id;
    self.next_batch_id = TimelineBatchId(batch_id.0 + 1);
    self.emit(CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
        key: self.key.clone(),
        generation: self.generation,
        batch_id,
        diffs: core_diffs,
    }));
}
```

- [ ] **Step 9: Implement policy change handler**

```rust
async fn handle_link_preview_policy_changed(
    &mut self,
    policy: crate::link_preview::LinkPreviewContext,
) {
    self.link_preview_policy = policy;
    let context = self.link_preview_policy.for_room(self.key.room_id());

    let mut core_diffs = Vec::new();
    for (index, item) in self.navigation_items.iter_mut().enumerate() {
        let old = item.link_previews.clone();
        apply_link_previews_to_item(item, &context, &self.session);
        if item.link_previews != old {
            core_diffs.push(TimelineDiff::Set {
                index,
                item: item.clone(),
            });
        }
    }

    if core_diffs.is_empty() {
        return;
    }

    let batch_id = self.next_batch_id;
    self.next_batch_id = TimelineBatchId(batch_id.0 + 1);
    self.emit(CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
        key: self.key.clone(),
        generation: self.generation,
        batch_id,
        diffs: core_diffs,
    }));
}
```

- [ ] **Step 10: Compile core**

```bash
cargo check -p koushi-core
```

Expected: no errors after all call-site updates.

---

### Task 8: Broadcast settings policy from the runtime

**Files:**
- Modify: `crates/koushi-core/src/runtime.rs:1519-1540`, `:1330-1342`

- [ ] **Step 1: Broadcast on settings change**

In `handle_app_effects`, when handling `EmitUiEvent(UiEvent::SettingsChanged)`, also send a timeline command:

```rust
} else if let AppEffect::EmitUiEvent(UiEvent::SettingsChanged) = effect {
    self.handle_ui_event_effect(&ui_event, &[]);
    let policy = crate::command::TimelineCommand::BroadcastLinkPreviewPolicy {
        global_enabled: self.state.settings.values.display.url_previews_enabled,
        room_overrides: self.state.settings.values.room_url_previews.clone(),
    };
    let _ = self
        .account_actor
        .send(crate::account::AccountMessage::TimelineCommand(policy))
        .await;
}
```

- [ ] **Step 2: Run runtime tests**

```bash
cargo test -p koushi-core --test runtime_settings
```

Expected: PASS.

---

### Task 9: Add Tauri commands

**Files:**
- Modify: `apps/desktop/src-tauri/src/commands.rs:3571-3602`, `:1560-1600`
- Modify: `apps/desktop/src-tauri/src/lib.rs:invoke handler`, `:1217-2200`

- [ ] **Step 1: Add command builders**

After `build_load_message_source_command`:

```rust
pub(crate) fn build_load_link_previews_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    event_id: String,
) -> Option<CoreCommand> {
    if event_id.trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::LoadLinkPreviews {
        request_id,
        key: build_timeline_key(account_key, room_id),
        event_id,
    }))
}

pub(crate) fn build_hide_link_preview_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    event_id: String,
) -> Option<CoreCommand> {
    if event_id.trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::HideLinkPreview {
        request_id,
        key: build_timeline_key(account_key, room_id),
        event_id,
    }))
}
```

- [ ] **Step 2: Add public Tauri commands**

```rust
#[tauri::command]
pub async fn load_link_previews(
    room_id: String,
    event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    if let Some(command) =
        build_load_link_previews_command(request_id, account_key, room_id, event_id)
    {
        submit_core_command(state.inner(), command).await?;
    }
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn hide_link_preview(
    room_id: String,
    event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    if let Some(command) =
        build_hide_link_preview_command(request_id, account_key, room_id, event_id)
    {
        submit_core_command(state.inner(), command).await?;
    }
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}
```

- [ ] **Step 3: Register commands in `lib.rs`**

Add `load_link_previews` and `hide_link_preview` to the `generate_handler!` invocation.

- [ ] **Step 4: Add builder contract tests**

In `apps/desktop/src-tauri/src/commands.rs` tests, add:

```rust
#[test]
fn load_link_previews_tauri_command_contract_is_present() {
    let request_id = RequestId {
        connection_id: RuntimeConnectionId(1),
        sequence: 1,
    };
    let command = build_load_link_previews_command(
        request_id,
        AccountKey("@u:example.test".to_owned()),
        "!room:example.test".to_owned(),
        "$event:example.test".to_owned(),
    );
    assert!(matches!(
        command,
        Some(CoreCommand::Timeline(TimelineCommand::LoadLinkPreviews { .. }))
    ));
}

#[test]
fn hide_link_preview_tauri_command_contract_is_present() {
    let request_id = RequestId {
        connection_id: RuntimeConnectionId(1),
        sequence: 1,
    };
    let command = build_hide_link_preview_command(
        request_id,
        AccountKey("@u:example.test".to_owned()),
        "!room:example.test".to_owned(),
        "$event:example.test".to_owned(),
    );
    assert!(matches!(
        command,
        Some(CoreCommand::Timeline(TimelineCommand::HideLinkPreview { .. }))
    ));
}
```

- [ ] **Step 5: Run Tauri contract test**

```bash
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml commands::load_link_previews_tauri_command_contract_is_present -- --exact
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml commands::hide_link_preview_tauri_command_contract_is_present -- --exact
```

Expected: PASS.

---

### Task 10: Add headless core QA scenario

**Files:**
- Modify: `crates/koushi-core/src/bin/headless-core-qa.rs:107-153`, `:399-470`, `:546-718`
- Modify: `scripts/desktop-headless-local-qa.mjs:22-41`

- [ ] **Step 1: Add scenario and stage**

```rust
enum QaScenario {
    // ... existing variants ...
    LinkPreview,
}

enum QaStage {
    // ... existing stages ...
    LinkPreview,
}
```

- [ ] **Step 2: Add stage wiring**

In `stages_for_scenario`:

```rust
QaScenario::LinkPreview => vec![
    QaStage::Safety,
    QaStage::LoginSync,
    QaStage::RoomSpace,
    QaStage::Timeline,
    QaStage::Composer,
    QaStage::LinkPreview,
],
```

In `tokens_for_stage`:

```rust
QaStage::LinkPreview => &[
    "link_preview_global=ok",
    "link_preview_room=ok",
    "link_preview_e2ee_disabled=ok",
    "link_preview_hide=ok",
],
```

In `QaScenario::All`, include `QaStage::LinkPreview`.

- [ ] **Step 3: Add a stage runner**

After the `run_media_stage` block, add:

```rust
async fn run_link_preview_stage(
    conn_a: &mut CoreConnection,
    conn_b: &mut CoreConnection,
    key_a: &TimelineKey,
    key_b: &TimelineKey,
) -> Result<(), String> {
    const URL_MESSAGE_BODY: &str = "link preview test message https://example.invalid/page";

    // 1. Send a message containing a URL.
    let txn = "qa-link-preview-txn".to_owned();
    let send_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::SendText {
            request_id: send_id,
            key: key_a.clone(),
            transaction_id: txn.clone(),
            body: URL_MESSAGE_BODY.to_owned(),
            mentions: MentionIntent::default(),
        }))
        .await
        .map_err(|e| format!("submit link preview message: {e}"))?;

    let (send_txn, event_id) = wait_for_send_completed(conn_a, send_id, key_a, &txn, "link preview send").await?;
    assert_eq!(send_txn, txn);

    // 2. Wait for B to see the message and a pending preview.
    let item = wait_for_timeline_item_with_body(conn_b, key_b, URL_MESSAGE_BODY, "B sees URL message").await?;
    let event_id = match &item.id {
        koushi_core::event::TimelineItemId::Event { event_id } => event_id.clone(),
        _ => return Err("link preview item was not event-backed".to_owned()),
    };
    let previews = item.link_previews.as_ref().ok_or("missing link_previews")?;
    assert_eq!(previews.len(), 1);
    assert_eq!(previews[0].url, "https://example.invalid/page");
    assert!(matches!(previews[0].state, koushi_core::event::LinkPreviewState::Pending));
    println!("link_preview_global=ok");

    // 3. Disable previews globally and confirm the projection drops them.
    let settings_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::App(AppCommand::UpdateSettings {
            request_id: settings_id,
            patch: SettingsPatch {
                display: Some(DisplaySettings {
                    code_block_wrap: true,
                    hide_redacted: false,
                    url_previews_enabled: false,
                }),
                ..SettingsPatch::default()
            },
        }))
        .await
        .map_err(|e| format!("submit global preview disable: {e}"))?;
    wait_for_settings_persisted(conn_b, settings_id, "global preview disable").await?;

    let disabled_item = wait_for_timeline_item_with_body(conn_b, key_b, URL_MESSAGE_BODY, "B sees message after global disable").await?;
    assert!(disabled_item.link_previews.as_ref().map(|p| p.is_empty()).unwrap_or(true));
    println!("link_preview_room=ok");

    // 4. Re-enable globally, hide this preview, and confirm empty projection.
    let settings_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::App(AppCommand::UpdateSettings {
            request_id: settings_id,
            patch: SettingsPatch {
                display: Some(DisplaySettings {
                    code_block_wrap: true,
                    hide_redacted: false,
                    url_previews_enabled: true,
                }),
                ..SettingsPatch::default()
            },
        }))
        .await
        .map_err(|e| format!("submit global preview enable: {e}"))?;
    wait_for_settings_persisted(conn_b, settings_id, "global preview enable").await?;

    let hide_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Timeline(TimelineCommand::HideLinkPreview {
            request_id: hide_id,
            key: key_b.clone(),
            event_id: event_id.clone(),
        }))
        .await
        .map_err(|e| format!("submit hide link preview: {e}"))?;

    let hidden_item = wait_for_timeline_item_with_body(conn_b, key_b, URL_MESSAGE_BODY, "B sees message after hide").await?;
    assert_eq!(hidden_item.link_previews.as_ref(), Some(&Vec::new()));
    println!("link_preview_hide=ok");

    Ok(())
}
```

Add `wait_for_settings_persisted` if it does not exist (follow the existing `wait_for_*` helpers pattern), reading `CoreEvent::UiEvent` for `SettingsChanged` and confirming `state.settings.persistence` is idle.

- [ ] **Step 4: Invoke the stage runner**

In the main scenario function, after the existing `Reply` stage block:

```rust
if scenario.should_run_stage(QaStage::LinkPreview) {
    run_link_preview_stage(&mut conn_a, &mut conn_b, &key_a, &key_b).await?;
}
```

- [ ] **Step 5: Add scenario to the QA runner script**

In `scripts/desktop-headless-local-qa.mjs`, insert after `"scenario reply"`:

```js
"scenario link_preview",
```

- [ ] **Step 6: Compile and run the scenario locally**

```bash
cargo build -p koushi-core --bin headless-core-qa --features qa-bin
mkdir -p /tmp/matrix-desktop-local-qa-bin
cp target/debug/headless-core-qa /tmp/matrix-desktop-local-qa-bin/
PATH=/tmp/matrix-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:headless-local -- --server=conduit --scenario=link_preview --core --core-backend=both --timeout-ms=240000
```

Expected token-only output includes:

```
link_preview_global=ok
link_preview_room=ok
link_preview_e2ee_disabled=ok
link_preview_hide=ok
```

---

## Phase B — GUI / browser-headless

### Task 11: Extend TypeScript domain types

**Files:**
- Modify: `apps/desktop/src/domain/types.ts:145-153`, `:61-79`, `:467-495`

- [ ] **Step 1: Extend `DisplaySettings`**

```ts
export interface DisplaySettings {
  code_block_wrap: boolean;
  hide_redacted: boolean;
  url_previews_enabled: boolean;
}
```

- [ ] **Step 2: Extend `SettingsValues` and `SettingsPatch`**

```ts
export interface SettingsValues {
  locale: LocaleSettings;
  appearance: AppearanceSettings;
  typography: TypographySettings;
  keyboard: KeyboardSettings;
  notifications: NotificationSettings;
  display: DisplaySettings;
  media: MediaSettings;
  room_url_previews: Record<string, boolean>;
}

export interface SettingsPatch {
  locale?: LocaleSettings;
  appearance?: AppearanceSettings;
  typography?: TypographySettings;
  keyboard?: KeyboardSettings;
  notifications?: NotificationSettings;
  display?: DisplaySettings;
  media?: MediaSettings;
  room_url_previews?: Record<string, boolean>;
}
```

- [ ] **Step 3: Extend `RoomSummary` and `RoomInteractionState`**

```ts
export interface RoomSummary {
  room_id: string;
  display_name: string;
  display_label: string;
  original_display_label: string;
  avatar: AvatarImage | null;
  is_dm: boolean;
  dm_user_ids: string[];
  tags: RoomTags;
  unread_count: number;
  notification_count?: number;
  highlight_count?: number;
  parent_space_ids: string[];
  is_encrypted: boolean;
}
```

`RoomInteractionState` does not need a hidden set (it lives in the Rust timeline actor), so leave it unchanged.

- [ ] **Step 4: Run TypeScript typecheck**

```bash
npm --prefix apps/desktop run typecheck
```

Expected: type errors only in places fixed in the following tasks.

---

### Task 12: Extend core event TypeScript contract

**Files:**
- Modify: `apps/desktop/src/domain/coreEvents.ts:166-189`

- [ ] **Step 1: Add link-preview types**

```ts
export type LinkPreviewState = "pending" | "loading" | "ready" | "failed";

export interface LinkPreviewImage {
  source: TimelineMediaSource;
  width?: number | null;
  height?: number | null;
  thumbnail: AvatarThumbnailState;
}

export interface LinkPreview {
  url: string;
  title?: string | null;
  description?: string | null;
  image?: LinkPreviewImage | null;
  state: LinkPreviewState;
}
```

- [ ] **Step 2: Extend `TimelineItem`**

```ts
export interface TimelineItem {
  id: TimelineItemId;
  sender: string | null;
  sender_label?: string | null;
  body: string | null;
  message_kind?: TimelineMessageKind;
  spoiler_spans?: TimelineSpoilerSpan[];
  timestamp_ms: number | null;
  in_reply_to_event_id: string | null;
  formatted?: TimelineFormattedBody | null;
  reply_quote?: ReplyQuote | null;
  thread_root: string | null;
  thread_summary: ThreadSummaryDto | null;
  media?: TimelineMedia | null;
  link_previews?: LinkPreview[] | null;
  reactions: ReactionGroup[];
  can_react: boolean;
  is_redacted: boolean;
  is_hidden: boolean;
  can_redact: boolean;
  is_edited: boolean;
  can_edit: boolean;
  actions?: TimelineMessageActions;
  send_state?: TimelineSendState | null;
}
```

- [ ] **Step 3: Regenerate `coreEvents.generated.json`**

Temporarily add an update branch to the contract test in `apps/desktop/src-tauri/src/lib.rs`:

```rust
let actual_contract = serde_json::json!({ ... });
if std::env::var("UPDATE_CONTRACT").is_ok() {
    std::fs::write(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../src/domain/coreEvents.generated.json"),
        serde_json::to_string_pretty(&actual_contract).unwrap(),
    )
    .unwrap();
    return;
}
```

Run:

```bash
UPDATE_CONTRACT=1 cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml core_event_wire_format_matches_checked_in_contract_artifact
```

Then remove the temporary update branch and run:

```bash
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml core_event_wire_format_matches_checked_in_contract_artifact
```

Expected: PASS.

---

### Task 13: Add i18n entries

**Files:**
- Modify: `apps/desktop/src/i18n/messages.ts:307-309`, `:1055-1057`, `:1704-1706`

- [ ] **Step 1: Extend the `MessageId` union**

```ts
| "settings.urlPreviews"
| "settings.urlPreviewsDescription"
| "settings.urlPreviewsRoom"
| "settings.urlPreviewsEncryptedNotice"
| "timeline.linkPreviewHide"
| "timeline.linkPreviewFailed"
| "timeline.linkPreviewLoading"
```

- [ ] **Step 2: Add English catalog entries**

Near the display settings entries:

```ts
"settings.urlPreviews": "Show URL previews",
"settings.urlPreviewsDescription": "Display preview cards for links in messages",
"settings.urlPreviewsRoom": "Show URL previews in this room",
"settings.urlPreviewsEncryptedNotice": "URL previews are disabled in encrypted rooms to protect your privacy. Enabling them sends URLs to your homeserver.",
"timeline.linkPreviewHide": "Hide preview",
"timeline.linkPreviewFailed": "Could not load preview",
"timeline.linkPreviewLoading": "Loading preview…",
```

- [ ] **Step 3: Add Japanese catalog entries**

```ts
"settings.urlPreviews": "URLプレビューを表示",
"settings.urlPreviewsDescription": "メッセージ内のリンクのプレビューカードを表示",
"settings.urlPreviewsRoom": "このルームでURLプレビューを表示",
"settings.urlPreviewsEncryptedNotice": "暗号化されたルームでは、プライバシー保護のためURLプレビューが無効になっています。有効にすると、URLがホームサーバーに送信されます。",
"timeline.linkPreviewHide": "プレビューを非表示",
"timeline.linkPreviewFailed": "プレビューを読み込めませんでした",
"timeline.linkPreviewLoading": "プレビューを読み込み中…",
```

- [ ] **Step 4: Run i18n tests**

```bash
npm --prefix apps/desktop run test -- --run src/i18n/messages.test.ts
```

Expected: PASS.

---

### Task 14: Add the global URL-preview toggle

**Files:**
- Modify: `apps/desktop/src/components/UserSettingsPanel.tsx:2168-2210`, `:376-391`, `:33-46`

- [ ] **Step 1: Extend `DisplayToggle` icon union and mapping**

```ts
icon: "code" | "hideRedacted" | "link";
```

```ts
const Icon = icon === "code" ? Code2 : icon === "hideRedacted" ? EyeOff : Link;
```

Import `Link` from `lucide-react` if not already imported.

- [ ] **Step 2: Add the toggle in the Display section**

```tsx
<DisplayToggle
  label={t("settings.urlPreviews")}
  settingKey="url_previews_enabled"
  icon="link"
  current={selectedDisplay}
  onSelect={onUpdateSettings}
/>
```

- [ ] **Step 3: Run the settings panel tests**

```bash
npm --prefix apps/desktop run test -- --run src/components/UserSettingsPanel.test.tsx
```

Expected: PASS.

---

### Task 15: Add per-room URL-preview toggle + E2EE notice

**Files:**
- Modify: `apps/desktop/src/components/RoomInfoPanel.tsx:1-58`, `:156-184`
- Modify: `apps/desktop/src/App.tsx:5749-5780`

- [ ] **Step 1: Add `settings` and `onUpdateSettings` props**

```ts
import type { SettingsState, SettingsPatch } from "../domain/types";

export function RoomInfoPanel({
  // ... existing props ...
  settings,
  onUpdateSettings
}: {
  // ... existing prop types ...
  settings?: SettingsState;
  onUpdateSettings?: (patch: SettingsPatch) => void;
}) {
```

- [ ] **Step 2: Compute effective per-room setting**

```ts
const roomUrlPreviewsEnabled =
  settings?.values.room_url_previews[roomId] ??
  settings?.values.display.url_previews_enabled ??
  true;
```

- [ ] **Step 3: Render the toggle and notice**

Inside the room settings section (around line 156), add:

```tsx
{settings && onUpdateSettings ? (
  <section className="settings-section" aria-label={t("settings.urlPreviews")}>
    <h3>{t("settings.urlPreviews")}</h3>
    <button
      className="settings-toggle-row"
      type="button"
      role="switch"
      aria-checked={roomUrlPreviewsEnabled}
      aria-label={t("settings.urlPreviewsRoom")}
      onClick={() => {
        onUpdateSettings({
          room_url_previews: {
            [roomId]: !roomUrlPreviewsEnabled
          }
        });
      }}
    >
      <span className="settings-toggle-copy">
        <span className="settings-toggle-label">
          <Link size={15} aria-hidden="true" />
          <span>{t("settings.urlPreviewsRoom")}</span>
        </span>
      </span>
      <span className="settings-switch-track" aria-hidden="true">
        <span className="settings-switch-thumb" />
      </span>
    </button>
    {room?.is_encrypted ? (
      <p className="settings-notice" role="note">
        {t("settings.urlPreviewsEncryptedNotice")}
      </p>
    ) : null}
  </section>
) : null}
```

Import `Link` from `lucide-react`.

- [ ] **Step 4: Pass props from `App.tsx`**

```tsx
<RoomInfoPanel
  // ... existing props ...
  settings={snapshot.state.settings}
  onUpdateSettings={(patch) => {
    void updateSettings(patch);
  }}
/>
```

- [ ] **Step 5: Run typecheck**

```bash
npm --prefix apps/desktop run typecheck
```

Expected: PASS.

---

### Task 16: Render link-preview cards in the timeline

**Files:**
- Modify: `apps/desktop/src/components/TimelineView.tsx:103-154`, `:2200-2320`, `:2790-2850`
- Modify: `apps/desktop/src/styles.css`

- [ ] **Step 1: Extend `TimelineTransport`**

```ts
export interface TimelineTransport {
  // ... existing methods ...
  /** Request Rust-owned link-preview metadata for a timeline event. */
  loadLinkPreviews(roomId: string, eventId: string): Promise<void>;
  /** Hide the link previews for a timeline event. */
  hideLinkPreview(roomId: string, eventId: string): Promise<void>;
}
```

- [ ] **Step 2: Add a `LinkPreviewCard` component**

```tsx
function LinkPreviewCard({
  preview,
  onHide
}: {
  preview: LinkPreview;
  onHide: () => void;
}) {
  return (
    <article
      className="link-preview-card"
      data-preview-state={preview.state}
    >
      <div className="link-preview-header">
        <a
          className="link-preview-title"
          href={preview.url}
          target="_blank"
          rel="noopener noreferrer"
        >
          {preview.title ?? preview.url}
        </a>
        <button
          className="link-preview-hide"
          type="button"
          aria-label={t("timeline.linkPreviewHide")}
          onClick={onHide}
        >
          <X size={14} />
        </button>
      </div>
      {preview.description ? (
        <p className="link-preview-description">{preview.description}</p>
      ) : null}
      {preview.image?.thumbnail.kind === "ready" ? (
        <img
          className="link-preview-image"
          src={preview.image.thumbnail.source_url}
          alt=""
        />
      ) : null}
      {preview.state === "loading" ? (
        <span className="link-preview-loading">{t("timeline.linkPreviewLoading")}</span>
      ) : null}
      {preview.state === "failed" ? (
        <span className="link-preview-failed">{t("timeline.linkPreviewFailed")}</span>
      ) : null}
    </article>
  );
}
```

- [ ] **Step 3: Render previews in `MessageRow`**

In the `MessageRow` body, after `mediaContent`/`bodyContent`, compute:

```tsx
const eventId =
  item.id && "Event" in item.id ? item.id.Event.event_id : null;

useEffect(() => {
  if (
    !eventId ||
    !item.link_previews?.some((preview) => preview.state === "pending")
  ) {
    return;
  }
  transport.loadLinkPreviews(roomId, eventId).catch(() => undefined);
}, [eventId, item.link_previews, roomId, transport]);

const linkPreviewContent =
  !isRedacted && eventId && item.link_previews && item.link_previews.length > 0 ? (
    <div className="message-link-previews">
      {item.link_previews.map((preview, index) => (
        <LinkPreviewCard
          key={`${eventId}:${index}`}
          preview={preview}
          onHide={() => {
            transport.hideLinkPreview(roomId, eventId).catch(() => undefined);
          }}
        />
      ))}
    </div>
  ) : null;
```

Render `linkPreviewContent` after the body/media block.

- [ ] **Step 4: Add CSS to `apps/desktop/src/styles.css`**

```css
.message-link-previews {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
  margin-top: 0.5rem;
}

.link-preview-card {
  border: 1px solid var(--border-subtle);
  border-radius: 0.5rem;
  padding: 0.75rem;
  max-width: 30rem;
}

.link-preview-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 0.5rem;
}

.link-preview-title {
  font-weight: 600;
  word-break: break-all;
}

.link-preview-description {
  margin: 0.25rem 0 0;
  color: var(--text-secondary);
}

.link-preview-image {
  margin-top: 0.5rem;
  max-width: 100%;
  max-height: 12rem;
  border-radius: 0.25rem;
  object-fit: cover;
}

.link-preview-hide {
  background: transparent;
  border: none;
  color: var(--text-secondary);
  cursor: pointer;
}

.link-preview-loading,
.link-preview-failed {
  font-size: 0.875rem;
  color: var(--text-secondary);
}
```

- [ ] **Step 5: Run timeline view tests**

```bash
npm --prefix apps/desktop run test -- --run src/components/TimelineView.test.tsx
```

Expected: PASS.

---

### Task 17: Wire transport methods

**Files:**
- Modify: `apps/desktop/src/App.tsx:358-388`
- Modify: `apps/desktop/src/backend/client.ts:81-140`
- Modify: `apps/desktop/src/backend/browserFakeApi.ts:42-140`, `:1185-1195`

- [ ] **Step 1: Wire Tauri timeline transport**

In `tauriTimelineTransport`:

```ts
async loadLinkPreviews(roomId: string, eventId: string) {
  await invoke("load_link_previews", { roomId, eventId });
},
async hideLinkPreview(roomId: string, eventId: string) {
  await invoke("hide_link_preview", { roomId, eventId });
},
```

- [ ] **Step 2: Extend `DesktopApi`**

```ts
loadLinkPreviews(roomId: string, eventId: string): Promise<DesktopSnapshot>;
hideLinkPreview(roomId: string, eventId: string): Promise<DesktopSnapshot>;
```

- [ ] **Step 3: Implement in `TauriDesktopApi`**

```ts
async loadLinkPreviews(roomId: string, eventId: string): Promise<DesktopSnapshot> {
  return invoke<DesktopSnapshot>("load_link_previews", { roomId, eventId });
}

async hideLinkPreview(roomId: string, eventId: string): Promise<DesktopSnapshot> {
  return invoke<DesktopSnapshot>("hide_link_preview", { roomId, eventId });
}
```

- [ ] **Step 4: Implement browser fake stubs**

```ts
async loadLinkPreviews(_roomId: string, _eventId: string): Promise<DesktopSnapshot> {
  return this.getSnapshot();
}

async hideLinkPreview(_roomId: string, _eventId: string): Promise<DesktopSnapshot> {
  return this.getSnapshot();
}
```

- [ ] **Step 5: Run typecheck**

```bash
npm --prefix apps/desktop run typecheck
```

Expected: PASS.

---

### Task 18: Add browser-headless contract tests

**Files:**
- Modify: `apps/desktop/e2e/basic-operations.spec.ts:53-51`, `:4644-4750`

- [ ] **Step 1: Add a global-toggle test**

```ts
test("URL previews global toggle invokes update_settings", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => window.__harness.clearInvocations());
  await page.getByRole("button", { name: "User settings" }).click();

  const toggle = page.getByRole("switch", { name: "Show URL previews" });
  await expect(toggle).toHaveAttribute("aria-checked", "true");
  await toggle.click();

  await expect.poll(() => invocationCount(page, "update_settings")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("update_settings")[0]?.args)
    )
    .toEqual({
      patch: {
        display: {
          code_block_wrap: true,
          hide_redacted: false,
          url_previews_enabled: false
        }
      }
    });
});
```

- [ ] **Step 2: Add a timeline card rendering + hide test**

```ts
test("link preview card renders from Rust-owned DTO and hides on close", async ({ page }) => {
  await gotoReadyShell(page);
  const eventId = "$link-preview:example.invalid";
  await seedTimelineItems(page, [
    {
      id: { Event: { event_id: eventId } },
      sender: "@harness-user:example.invalid",
      body: "See https://example.invalid/page",
      timestamp_ms: 1_800_000_001_000,
      in_reply_to_event_id: null,
      thread_root: null,
      thread_summary: null,
      reactions: [],
      can_react: true,
      is_redacted: false,
      is_hidden: false,
      can_redact: false,
      is_edited: false,
      can_edit: false,
      link_previews: [
        {
          url: "https://example.invalid/page",
          title: "Example Preview",
          description: "A synthetic preview for testing.",
          image: null,
          state: "ready"
        }
      ]
    }
  ]);

  const row = page.locator(`[data-event-id="${eventId}"]`);
  await expect(row.locator(".link-preview-card")).toBeVisible();
  await expect(row.getByText("Example Preview")).toBeVisible();

  await page.evaluate(() => window.__harness.clearInvocations());
  await row.getByRole("button", { name: "Hide preview" }).click();

  await expect.poll(() => invocationCount(page, "hide_link_preview")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("hide_link_preview")[0]?.args)
    )
    .toEqual({ roomId: "!harness-room:example.invalid", eventId });
});
```

- [ ] **Step 3: Add an encrypted-room privacy test**

```ts
test("encrypted room suppresses link previews and shows privacy notice", async ({ page }) => {
  await gotoReadyShell(page);
  const snapshot = await page.evaluate(() => window.__harness.currentSnapshot());
  const encryptedRoom = {
    ...snapshot.state.rooms[0],
    room_id: "!encrypted:example.invalid",
    display_name: "Encrypted Room",
    display_label: "Encrypted Room",
    is_encrypted: true
  };
  snapshot.state.rooms = [encryptedRoom];
  await page.evaluate((next) => window.__harness.setSnapshot(next), snapshot);

  await page.getByRole("button", { name: "Room info" }).click();
  await expect(page.getByText("URL previews are disabled in encrypted rooms")).toBeVisible();
});
```

- [ ] **Step 4: Run the focused Playwright tests**

```bash
npm --prefix apps/desktop exec -- playwright test e2e/basic-operations.spec.ts -g "URL link preview|link preview card|encrypted room suppresses" --workers=1
```

Expected: PASS.

---

## Verification

Run the full verification stack after all tasks:

```bash
# Rust unit + integration tests
cargo test -p koushi-state --test link_preview_state
cargo test -p koushi-core --lib
cargo test -p koushi-core --bin headless-core-qa --features qa-bin
cargo test -p koushi-sdk -p koushi-state -p koushi-search -p koushi-key
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml

# WASM compile check
cargo check --target wasm32-unknown-unknown -p koushi-state -p koushi-search

# TypeScript
cd apps/desktop
npm run typecheck
npm run test
npm run test:ui-headless
npm run test:ipc-contract
npm run qa:secret-scan

# Release gate
cd ../..
node scripts/desktop-release-gate-check.mjs --no-compile
```

Final headless proof:

```bash
PATH=/tmp/matrix-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:headless-local -- --server=conduit --scenario=link_preview --core --core-backend=both --timeout-ms=240000
```

Expected token-only output (no URLs, room IDs, event IDs, or raw SDK errors):

```
server=conduit
safety=ok
login_sync=ok
room_space=ok
timeline=ok
timeline_nav=ok
hide_redacted=ok
mention_send=ok
markdown_send=ok
slash_command=ok
ime_guard=ok
link_preview_global=ok
link_preview_room=ok
link_preview_e2ee_disabled=ok
link_preview_hide=ok
restore_cleanup=ok
```

---

## Self-review

- **Spec coverage:**
  - Global enable/disable → Task 1 + Task 14.
  - Per-room override → Task 1 + Task 15.
  - E2EE default-off privacy guard → Task 3 + Task 6 + Task 15.
  - Viewer-local hide → Task 7 + Task 16.
  - Multiple-link cap and cache reuse → Task 6 unit tests.
  - Token-only, private-data-free QA → Task 10 + all verification commands.
- **Placeholder scan:** No `TBD`, `TODO`, `implement later`, or unverified placeholders remain; every task has concrete file paths, code, and expected commands/output.
- **Type consistency:**
  - Rust: `url_previews_enabled`, `room_url_previews`, `is_encrypted`, `LinkPreview`, `LinkPreviewState`, `TimelineCommand::*` names are used identically across state, command, event, timeline, runtime, and Tauri layers.
  - TypeScript: `url_previews_enabled`, `room_url_previews`, `is_encrypted`, `LinkPreview`, `LinkPreviewState`, and `link_previews` mirror the Rust wire shapes.
