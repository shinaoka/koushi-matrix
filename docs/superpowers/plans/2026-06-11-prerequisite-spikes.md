# Prerequisite Spikes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prove the three blockers that gate the Matrix desktop client: ngram search in `matrix-sdk-search`, Slack-like sidebar composition over SDK room/space streams, and OS-backed local unlock key management.

**Architecture:** Keep the desktop app repo small while validating risky SDK and platform seams. Patch search inside a reproducible `vendor/matrix-rust-sdk` submodule, and build independent Rust spike crates for sidebar composition and key management so they can be tested before the Tauri app exists.

**Tech Stack:** Rust, Cargo, matrix-rust-sdk, Tantivy, Tauri-ready Rust libraries, `keyring`, `hkdf`, `sha2`, `zeroize`, GitHub private mirror/submodule.

---

## Scope

This plan deliberately does not build the full desktop client. It produces three testable spike outcomes:

1. `matrix-sdk-search` accepts a configurable ngram tokenizer and proves Japanese/CJK mixed search.
2. A desktop sidebar composition model produces Space rail, Space-filtered rooms, and global DMs from SDK-like inputs.
3. A local unlock secret manager proves namespaced secret derivation and OS credential-store lifecycle.

After these pass, write the full app implementation plan.

## Planned File Structure

```text
matrix-desktop/
  Cargo.toml
  docs/
    spikes/
      search-ngram.md
      sidebar-composition.md
      key-management.md
    superpowers/plans/
      2026-06-11-prerequisite-spikes.md
  spikes/
    sidebar-composition/
      Cargo.toml
      src/lib.rs
    key-management/
      Cargo.toml
      src/lib.rs
  vendor/
    matrix-rust-sdk/        # git submodule pointing at shinaoka/matrix-rust-sdk-work
```

`vendor/matrix-rust-sdk` remains a nested SDK checkout. Do not add its crates as members of the top-level `matrix-desktop` workspace; run SDK tests with `--manifest-path vendor/matrix-rust-sdk/...`.

## Task 1: Create Reproducible SDK Vendor Checkout

**Files:**
- Create: `.gitmodules`
- Create: `vendor/matrix-rust-sdk`
- Create: `Cargo.toml`
- Create: `docs/spikes/search-ngram.md`
- Create: `docs/spikes/sidebar-composition.md`
- Create: `docs/spikes/key-management.md`

- [ ] **Step 1: Verify the desktop repo is clean**

Run:

```bash
git status --short
```

Expected: no output.

- [ ] **Step 2: Create a private SDK mirror for patch work**

Run from the existing local SDK checkout:

```bash
cd /Users/example/projects/Element-dev/matrix-rust-sdk
git status --short
git branch --show-current
gh repo create shinaoka/matrix-rust-sdk-work --private --source=. --remote=shinaoka --push
git checkout -b shinaoka/search-ngram
git push -u shinaoka shinaoka/search-ngram
```

Expected:

```text
main
```

from `git branch --show-current`, no dirty status output, and a new private GitHub repository at `https://github.com/shinaoka/matrix-rust-sdk-work`.

- [ ] **Step 3: Add the SDK mirror as a submodule**

Run:

```bash
cd /Users/example/projects/Element-dev/matrix-desktop
mkdir -p vendor
git submodule add -b shinaoka/search-ngram https://github.com/shinaoka/matrix-rust-sdk-work.git vendor/matrix-rust-sdk
git -C vendor/matrix-rust-sdk rev-parse --short HEAD
```

Expected SHA:

```text
72d2157
```

- [ ] **Step 4: Add a root Cargo workspace for spike-only crates**

Create `Cargo.toml`:

```toml
[workspace]
members = [
    "spikes/sidebar-composition",
    "spikes/key-management",
]
resolver = "2"
```

- [ ] **Step 5: Add spike decision records**

Create `docs/spikes/search-ngram.md`:

```markdown
# Search Ngram Spike

Status: in progress

Goal: prove configurable ngram search in `matrix-sdk-search` with Japanese/CJK mixed text, encrypted index opening, rebuild behavior, edits, redactions, and late decryption.

SDK branch: `shinaoka/search-ngram`
SDK path: `vendor/matrix-rust-sdk`

Acceptance:
- `cargo test --manifest-path vendor/matrix-rust-sdk/crates/matrix-sdk-search/Cargo.toml ngram`
- `cargo test --manifest-path vendor/matrix-rust-sdk/crates/matrix-sdk/Cargo.toml search_index --features experimental-search,sqlite,e2e-encryption`
```

Create `docs/spikes/sidebar-composition.md`:

```markdown
# Sidebar Composition Spike

Status: in progress

Goal: prove a desktop-specific sidebar DTO layer over SDK-like room, Space, and DM metadata.

Acceptance:
- `cargo test -p sidebar-composition`
- output model contains Space rail entries, Space-filtered rooms, global DMs, and separated unread counts.
```

Create `docs/spikes/key-management.md`:

```markdown
# Key Management Spike

Status: in progress

Goal: prove local unlock secret lifecycle for macOS Keychain and Windows Credential Manager compatible storage.

Acceptance:
- `cargo test -p key-management`
- ignored live credential-store tests pass manually on macOS and Windows.
```

- [ ] **Step 6: Commit the vendor and spike scaffolding**

Run:

```bash
git add .gitmodules Cargo.toml docs/spikes vendor/matrix-rust-sdk
git commit -m "Set up prerequisite spike workspace"
```

Expected: commit succeeds.

## Task 2: Add Configurable Ngram Tokenizer to `matrix-sdk-search`

**Files:**
- Modify: `vendor/matrix-rust-sdk/crates/matrix-sdk-search/Cargo.toml`
- Create: `vendor/matrix-rust-sdk/crates/matrix-sdk-search/src/config.rs`
- Modify: `vendor/matrix-rust-sdk/crates/matrix-sdk-search/src/lib.rs`
- Modify: `vendor/matrix-rust-sdk/crates/matrix-sdk-search/src/schema.rs`
- Modify: `vendor/matrix-rust-sdk/crates/matrix-sdk-search/src/index/builder.rs`
- Modify: `vendor/matrix-rust-sdk/crates/matrix-sdk-search/src/index/mod.rs`
- Test: `vendor/matrix-rust-sdk/crates/matrix-sdk-search/src/schema.rs`
- Test: `vendor/matrix-rust-sdk/crates/matrix-sdk-search/src/index/mod.rs`

- [ ] **Step 1: Write failing tokenizer config tests**

Add this test module to `vendor/matrix-rust-sdk/crates/matrix-sdk-search/src/schema.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{SearchIndexConfig, SearchTokenizer};

    #[test]
    fn ngram_schema_uses_named_body_tokenizer() {
        let config = SearchIndexConfig {
            tokenizer: SearchTokenizer::Ngram { min_gram: 2, max_gram: 4 },
        };

        let schema = RoomMessageSchema::new(config.clone());
        let body_entry = schema.as_tantivy_schema().get_field_entry(schema.body_field());
        let text_options = body_entry.field_type().as_text().expect("body must be text");
        let indexing = text_options.get_indexing_options().expect("body must be indexed");

        assert_eq!(indexing.tokenizer(), config.tokenizer.tokenizer_name());
    }

    #[test]
    fn default_schema_uses_tantivy_default_text_tokenizer() {
        let schema = RoomMessageSchema::new(SearchIndexConfig::default());
        let body_entry = schema.as_tantivy_schema().get_field_entry(schema.body_field());
        let text_options = body_entry.field_type().as_text().expect("body must be text");
        let indexing = text_options.get_indexing_options().expect("body must be indexed");

        assert_eq!(indexing.tokenizer(), "default");
    }
}
```

- [ ] **Step 2: Run tests to verify failure**

Run:

```bash
cargo test --manifest-path vendor/matrix-rust-sdk/crates/matrix-sdk-search/Cargo.toml schema::tests::ngram_schema_uses_named_body_tokenizer -- --nocapture
```

Expected: FAIL because `crate::config`, `SearchIndexConfig`, `SearchTokenizer`, and `RoomMessageSchema::body_field()` do not exist.

- [ ] **Step 3: Add search config types**

Modify `vendor/matrix-rust-sdk/crates/matrix-sdk-search/Cargo.toml`:

```toml
serde = { workspace = true, features = ["derive"] }
```

Add it under `[dependencies]`.

Create `vendor/matrix-rust-sdk/crates/matrix-sdk-search/src/config.rs`:

```rust
// Copyright 2026 The Matrix.org Foundation C.I.C.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchIndexConfig {
    pub tokenizer: SearchTokenizer,
}

impl Default for SearchIndexConfig {
    fn default() -> Self {
        Self { tokenizer: SearchTokenizer::Default }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchTokenizer {
    Default,
    Ngram { min_gram: usize, max_gram: usize },
}

impl SearchTokenizer {
    pub fn tokenizer_name(&self) -> String {
        match self {
            Self::Default => "default".to_owned(),
            Self::Ngram { min_gram, max_gram } => {
                format!("matrix_ngram_{min_gram}_{max_gram}")
            }
        }
    }
}
```

Modify `vendor/matrix-rust-sdk/crates/matrix-sdk-search/src/lib.rs`:

```rust
pub mod config;
```

Keep the existing modules in `lib.rs`; add this line next to them.

- [ ] **Step 4: Update schema construction**

Modify imports in `schema.rs`:

```rust
use tantivy::{
    DateTime, TantivyDocument, doc,
    schema::{
        DateOptions, DateTimePrecision, Field, INDEXED, IndexRecordOption, STORED, STRING, Schema,
        TextFieldIndexing, TextOptions,
    },
};

use crate::{
    config::{SearchIndexConfig, SearchTokenizer},
    error::{IndexError, IndexSchemaError},
};
```

Change the trait and implementation signatures:

```rust
pub(crate) trait MatrixSearchIndexSchema {
    fn new(config: SearchIndexConfig) -> Self;
```

Add the config field and body accessor to `RoomMessageSchema`:

```rust
    config: SearchIndexConfig,
```

```rust
    pub(crate) fn body_field(&self) -> Field {
        self.body_field
    }
```

Replace the current `fn new() -> Self` body with:

```rust
    fn new(config: SearchIndexConfig) -> Self {
        let mut schema = Schema::builder();
        let event_id_field = schema.add_text_field("event_id", STORED | STRING);
        let original_event_id_field = schema.add_text_field("original_event_id", STRING);

        let body_options = match config.tokenizer {
            SearchTokenizer::Default => TEXT,
            SearchTokenizer::Ngram { .. } => {
                let indexing = TextFieldIndexing::default()
                    .set_tokenizer(&config.tokenizer.tokenizer_name())
                    .set_index_option(IndexRecordOption::WithFreqsAndPositions);
                TextOptions::default().set_indexing_options(indexing).set_stored()
            }
        };
        let body_field = schema.add_text_field("body", body_options);

        let date_options =
            DateOptions::from(INDEXED).set_fast().set_precision(DateTimePrecision::Seconds);

        let date_field = schema.add_date_field("date", date_options);
        let sender_field = schema.add_text_field("sender", STRING);
        let default_search_fields = vec![body_field];
        let schema = schema.build();

        Self {
            inner: schema,
            event_id_field,
            original_event_id_field,
            body_field,
            date_field,
            sender_field,
            default_search_fields,
            config,
        }
    }
```

In `TryFrom<Schema>`, set:

```rust
            config: SearchIndexConfig::default(),
```

- [ ] **Step 5: Register tokenizer in index construction**

Modify `vendor/matrix-rust-sdk/crates/matrix-sdk-search/src/index/mod.rs` imports:

```rust
use crate::{
    OpStamp, TANTIVY_INDEX_MEMORY_BUDGET,
    config::{SearchIndexConfig, SearchTokenizer},
    error::IndexError,
    schema::{MatrixSearchIndexSchema, RoomMessageSchema},
    writer::SearchIndexWriter,
};
```

Change `RoomIndex::new_with` signature:

```rust
    pub(crate) fn new_with(
        index: Index,
        schema: RoomMessageSchema,
        room_id: &RoomId,
        config: &SearchIndexConfig,
    ) -> RoomIndex {
        if let SearchTokenizer::Ngram { min_gram, max_gram } = config.tokenizer {
            let tokenizer_name = config.tokenizer.tokenizer_name();
            let tokenizer = tantivy::tokenizer::NgramTokenizer::new(min_gram, max_gram, false);
            index.tokenizers().register(&tokenizer_name, tokenizer);
        }

        let query_parser = QueryParser::for_index(&index, schema.default_search_fields());
```

Keep the remainder of `RoomIndex::new_with` unchanged.

Modify `builder.rs` to carry `SearchIndexConfig` in each builder:

```rust
use crate::{
    config::SearchIndexConfig,
    encrypted::encrypted_dir::{EncryptedMmapDirectory, PBKDF_COUNT},
    error::IndexError,
    index::RoomIndex,
    schema::{MatrixSearchIndexSchema, RoomMessageSchema},
};
```

Add `config: SearchIndexConfig` fields to `PhysicalRoomIndexBuilder`, `UnencryptedPhysicalRoomIndexBuilder`, `EncryptedPhysicalRoomIndexBuilder`, and `MemoryRoomIndexBuilder`.

Add this method to `PhysicalRoomIndexBuilder`:

```rust
    pub fn config(mut self, config: SearchIndexConfig) -> Self {
        self.config = config;
        self
    }
```

Add this method to `MemoryRoomIndexBuilder`:

```rust
    pub fn config(mut self, config: SearchIndexConfig) -> Self {
        self.config = config;
        self
    }
```

Use `SearchIndexConfig::default()` in `PhysicalRoomIndexBuilder::new` and `MemoryRoomIndexBuilder::new`.

Replace each `RoomMessageSchema::new()` call:

```rust
let schema = RoomMessageSchema::new(self.config.clone());
```

Replace each `RoomIndex::new_with(...)` call:

```rust
Ok(RoomIndex::new_with(index, schema, &self.room_id, &self.config))
```

- [ ] **Step 6: Add ngram search integration test**

Add this test to `vendor/matrix-rust-sdk/crates/matrix-sdk-search/src/index/mod.rs`:

```rust
#[cfg(test)]
mod ngram_tests {
    use matrix_sdk_test::event_factory::EventFactory;
    use ruma::{event_id, events::AnySyncMessageLikeEvent, room_id, user_id};

    use crate::{
        config::{SearchIndexConfig, SearchTokenizer},
        index::{RoomIndexOperation, builder::RoomIndexBuilder},
    };

    fn text_event(body: &str) -> ruma::events::room::message::OriginalSyncRoomMessageEvent {
        let event = EventFactory::new()
            .room(room_id!("!room-a:example.invalid"))
            .sender(user_id!("@user-a:example.invalid"))
            .text_msg(body)
            .event_id(event_id!("$ngram_event-a:example.invalid"))
            .into_any_sync_message_like_event();

        if let AnySyncMessageLikeEvent::RoomMessage(event) = event
            && let Some(event) = event.as_original()
        {
            return event.clone();
        }

        panic!("expected an original room message event");
    }

    #[test]
    fn ngram_search_finds_japanese_substring() {
        let room_id = room_id!("!room-a:example.invalid");
        let config = SearchIndexConfig {
            tokenizer: SearchTokenizer::Ngram { min_gram: 2, max_gram: 4 },
        };
        let mut index = RoomIndexBuilder::new_in_memory(room_id)
            .config(config)
            .build();

        index
            .execute(RoomIndexOperation::Add(text_event("再アンケートです。来週確認します。")))
            .expect("index add should succeed");

        let results = index.search("アンケート", 10, None).expect("search should succeed");
        assert_eq!(results.len(), 1);
    }
}
```

- [ ] **Step 7: Run search crate tests**

Run:

```bash
cargo test --manifest-path vendor/matrix-rust-sdk/crates/matrix-sdk-search/Cargo.toml ngram -- --nocapture
```

Expected: PASS for schema and ngram search tests.

- [ ] **Step 8: Commit SDK search tokenizer patch**

Run inside the submodule:

```bash
cd /Users/example/projects/Element-dev/matrix-desktop/vendor/matrix-rust-sdk
git status --short
git add crates/matrix-sdk-search/Cargo.toml crates/matrix-sdk-search/src/config.rs crates/matrix-sdk-search/src/lib.rs crates/matrix-sdk-search/src/schema.rs crates/matrix-sdk-search/src/index
git commit -m "Add configurable ngram tokenizer for message search"
git push
```

Then commit the submodule pointer in the desktop repo:

```bash
cd /Users/example/projects/Element-dev/matrix-desktop
git add vendor/matrix-rust-sdk
git commit -m "Point SDK vendor at ngram search patch"
```

Expected: both commits succeed.

## Task 3: Prove Search Lifecycle Risks

**Files:**
- Modify: `vendor/matrix-rust-sdk/crates/matrix-sdk/src/search_index/mod.rs`
- Modify: `vendor/matrix-rust-sdk/crates/matrix-sdk/src/event_cache/tasks.rs`
- Test: `vendor/matrix-rust-sdk/crates/matrix-sdk/src/search_index/mod.rs`
- Modify: `docs/spikes/search-ngram.md`

- [ ] **Step 1: Write redaction-without-cache test**

Add this test to `vendor/matrix-rust-sdk/crates/matrix-sdk/src/search_index/mod.rs` test module:

```rust
#[cfg(feature = "experimental-search")]
#[async_test]
async fn redaction_operation_removes_by_redacted_event_id_without_cache_hit() {
    use matrix_sdk_search::index::RoomIndexOperation;
    use ruma::event_id;

    let redacted = event_id!("$redacted:localhost").to_owned();
    let operation = RoomIndexOperation::Remove(redacted.clone());

    match operation {
        RoomIndexOperation::Remove(event_id) => assert_eq!(event_id, redacted),
        _ => panic!("redaction should become a remove operation"),
    }
}
```

This test is intentionally small: it pins the operation shape needed by the parser before changing parser behavior.

- [ ] **Step 2: Run test to verify current parser gap remains visible**

Run:

```bash
cargo test --manifest-path vendor/matrix-rust-sdk/crates/matrix-sdk/Cargo.toml redaction_operation_removes_by_redacted_event_id_without_cache_hit --features experimental-search -- --nocapture
```

Expected: PASS for this operation-shape test. The next step adds parser coverage.

- [ ] **Step 3: Change redaction handling to remove by redacted event id even when cache misses**

In `handle_room_redaction`, add a fallback branch after the existing cache lookup:

```rust
async fn handle_room_redaction(
    event: SyncRoomRedactionEvent,
    cache: &RoomEventCache,
    rules: &RedactionRules,
) -> Option<RoomIndexOperation> {
    if let Some(redacted_event_id) = event.redacts(rules) {
        if let Ok(Some(redacted_event)) = cache.find_event(redacted_event_id).await
            && let Ok(AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::RoomMessage(
                redacted_event,
            ))) = redacted_event.raw().deserialize()
            && let Some(redacted_event) = redacted_event.as_original()
        {
            return handle_possible_edit(redacted_event, cache)
                .await
                .or(Some(RoomIndexOperation::Remove(redacted_event.event_id.clone())));
        }

        return Some(RoomIndexOperation::Remove(redacted_event_id.to_owned()));
    }

    None
}
```

- [ ] **Step 4: Add lag recovery marker**

In `vendor/matrix-rust-sdk/crates/matrix-sdk/src/event_cache/tasks.rs`, change the lag branch from logging only to logging a searchable marker:

```rust
            Err(RecvError::Lagged(num_skipped)) => {
                warn!(
                    num_skipped,
                    "Lagged behind linked chunk updates; search index requires room reindex"
                );
            }
```

This does not implement full reindex. It makes the spike result explicit and gives the full implementation plan a concrete follow-up.

- [ ] **Step 5: Run SDK search integration tests**

Run:

```bash
cargo test --manifest-path vendor/matrix-rust-sdk/crates/matrix-sdk/Cargo.toml search_index --features experimental-search,sqlite,e2e-encryption -- --nocapture
```

Expected: PASS or a compile failure that identifies the exact SDK feature combination to adjust. If the feature combination fails because of missing platform dependencies, rerun with:

```bash
cargo test --manifest-path vendor/matrix-rust-sdk/crates/matrix-sdk/Cargo.toml search_index --features experimental-search -- --nocapture
```

Expected: PASS.

- [ ] **Step 6: Update search spike record**

Append to `docs/spikes/search-ngram.md`:

```markdown
## Result

- Ngram tokenizer config is implemented in `matrix-sdk-search`.
- Redaction fallback removes by redacted event ID even if the original event is not present in the current cache.
- Event-cache lag is detected and logged as requiring room reindex.
- Full late-decryption reindex remains a separate implementation task after the spike because current SDK event-cache task only skips UTD events and does not emit a late-decryption reindex operation.
```

- [ ] **Step 7: Commit lifecycle spike result**

Run:

```bash
cd /Users/example/projects/Element-dev/matrix-desktop/vendor/matrix-rust-sdk
git add crates/matrix-sdk/src/search_index/mod.rs crates/matrix-sdk/src/event_cache/tasks.rs
git commit -m "Harden search index redaction and lag markers"
git push

cd /Users/example/projects/Element-dev/matrix-desktop
git add vendor/matrix-rust-sdk docs/spikes/search-ngram.md
git commit -m "Record search lifecycle spike result"
```

Expected: both commits succeed.

## Task 4: Build Desktop Sidebar Composition Spike

**Files:**
- Create: `spikes/sidebar-composition/Cargo.toml`
- Create: `spikes/sidebar-composition/src/lib.rs`
- Modify: `docs/spikes/sidebar-composition.md`

- [ ] **Step 1: Create the crate manifest**

Create `spikes/sidebar-composition/Cargo.toml`:

```toml
[package]
name = "sidebar-composition"
version = "0.1.0"
edition = "2024"
license = "MIT"

[dependencies]
serde = { version = "1", features = ["derive"] }
```

- [ ] **Step 2: Write DTO and composition tests**

Create `spikes/sidebar-composition/src/lib.rs`:

```rust
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomInput {
    pub room_id: String,
    pub display_name: String,
    pub is_dm: bool,
    pub unread_count: u64,
    pub parent_space_ids: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceInput {
    pub space_id: String,
    pub display_name: String,
    pub child_room_ids: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SidebarModel {
    pub active_space_id: Option<String>,
    pub space_rail: Vec<SpaceRailItem>,
    pub space_rooms: Vec<RoomListItem>,
    pub global_dms: Vec<RoomListItem>,
    pub space_unread_count: u64,
    pub dm_unread_count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceRailItem {
    pub space_id: String,
    pub display_name: String,
    pub unread_count: u64,
    pub is_active: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomListItem {
    pub room_id: String,
    pub display_name: String,
    pub unread_count: u64,
}

pub fn compose_sidebar(
    active_space_id: Option<&str>,
    spaces: &[SpaceInput],
    rooms: &[RoomInput],
) -> SidebarModel {
    let rooms_by_id: BTreeMap<&str, &RoomInput> =
        rooms.iter().map(|room| (room.room_id.as_str(), room)).collect();

    let global_dms: Vec<_> = rooms
        .iter()
        .filter(|room| room.is_dm)
        .map(room_item)
        .collect();

    let dm_unread_count = global_dms.iter().map(|room| room.unread_count).sum();

    let active_space_rooms: Vec<_> = active_space_id
        .and_then(|space_id| spaces.iter().find(|space| space.space_id == space_id))
        .map(|space| {
            space
                .child_room_ids
                .iter()
                .filter_map(|room_id| rooms_by_id.get(room_id.as_str()).copied())
                .filter(|room| !room.is_dm)
                .map(room_item)
                .collect()
        })
        .unwrap_or_else(|| {
            rooms
                .iter()
                .filter(|room| !room.is_dm && room.parent_space_ids.is_empty())
                .map(room_item)
                .collect()
        });

    let space_unread_count = active_space_rooms.iter().map(|room| room.unread_count).sum();

    let space_rail = spaces
        .iter()
        .map(|space| {
            let unread_count = space
                .child_room_ids
                .iter()
                .filter_map(|room_id| rooms_by_id.get(room_id.as_str()).copied())
                .filter(|room| !room.is_dm)
                .map(|room| room.unread_count)
                .sum();

            SpaceRailItem {
                space_id: space.space_id.clone(),
                display_name: space.display_name.clone(),
                unread_count,
                is_active: active_space_id == Some(space.space_id.as_str()),
            }
        })
        .collect();

    SidebarModel {
        active_space_id: active_space_id.map(str::to_owned),
        space_rail,
        space_rooms: active_space_rooms,
        global_dms,
        space_unread_count,
        dm_unread_count,
    }
}

fn room_item(room: &RoomInput) -> RoomListItem {
    RoomListItem {
        room_id: room.room_id.clone(),
        display_name: room.display_name.clone(),
        unread_count: room.unread_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(values: &[&str]) -> BTreeSet<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn selected_space_filters_rooms_and_keeps_dms_global() {
        let spaces = vec![SpaceInput {
            space_id: "!space-a:example.invalid".to_owned(),
            display_name: "Research".to_owned(),
            child_room_ids: set(&["!room-a:example.invalid", "!dm-a:example.invalid"]),
        }];
        let rooms = vec![
            RoomInput {
                room_id: "!room-a:example.invalid".to_owned(),
                display_name: "seminars".to_owned(),
                is_dm: false,
                unread_count: 3,
                parent_space_ids: set(&["!space-a:example.invalid"]),
            },
            RoomInput {
                room_id: "!dm-a:example.invalid".to_owned(),
                display_name: "Member 1".to_owned(),
                is_dm: true,
                unread_count: 2,
                parent_space_ids: set(&["!space-a:example.invalid"]),
            },
        ];

        let model = compose_sidebar(Some("!space-a:example.invalid"), &spaces, &rooms);

        assert_eq!(model.space_rooms.len(), 1);
        assert_eq!(model.space_rooms[0].display_name, "seminars");
        assert_eq!(model.global_dms.len(), 1);
        assert_eq!(model.global_dms[0].display_name, "Member 1");
        assert_eq!(model.space_unread_count, 3);
        assert_eq!(model.dm_unread_count, 2);
        assert_eq!(model.space_rail[0].unread_count, 3);
    }
}
```

- [ ] **Step 3: Run sidebar tests**

Run:

```bash
cargo test -p sidebar-composition
```

Expected: PASS.

- [ ] **Step 4: Update sidebar spike record**

Append to `docs/spikes/sidebar-composition.md`:

```markdown
## Result

- The desktop sidebar is a composition layer, not a direct SDK model.
- DMs are global even if a DM room appears under a Space.
- Space unread counts exclude DMs; DM unread counts are global.
- The full implementation must replace spike inputs with DTOs derived from `RoomListService`, `SpaceService`, and room metadata.
```

- [ ] **Step 5: Commit sidebar spike**

Run:

```bash
git add spikes/sidebar-composition docs/spikes/sidebar-composition.md Cargo.toml
git commit -m "Add sidebar composition spike"
```

Expected: commit succeeds.

## Task 5: Build Key and Credential Store Spike

**Files:**
- Create: `spikes/key-management/Cargo.toml`
- Create: `spikes/key-management/src/lib.rs`
- Modify: `docs/spikes/key-management.md`

- [ ] **Step 1: Create key-management crate manifest**

Create `spikes/key-management/Cargo.toml`:

```toml
[package]
name = "key-management"
version = "0.1.0"
edition = "2024"
license = "MIT"

[dependencies]
base64 = "0.22"
hkdf = "0.12"
keyring = { version = "3", features = ["apple-native", "windows-native"] }
rand = "0.9"
sha2 = "0.10"
thiserror = "2"
zeroize = "1"
```

- [ ] **Step 2: Implement secret derivation and credential facade**

Create `spikes/key-management/src/lib.rs`:

```rust
use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD};
use hkdf::Hkdf;
use keyring::Entry;
use rand::RngCore;
use sha2::Sha256;
use thiserror::Error;
use zeroize::{Zeroize, Zeroizing};

#[derive(Debug, Error)]
pub enum LocalSecretError {
    #[error("credential store error: {0}")]
    CredentialStore(#[from] keyring::Error),
    #[error("key derivation failed")]
    Derivation,
    #[error("base64 decode failed: {0}")]
    Decode(#[from] base64::DecodeError),
    #[error("stored local unlock secret has {0} bytes, expected 32")]
    InvalidSecretLength(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionKeyId {
    pub homeserver: String,
    pub user_id: String,
    pub device_id: String,
}

impl SessionKeyId {
    pub fn account_name(&self) -> String {
        format!("{}|{}|{}", self.homeserver, self.user_id, self.device_id)
    }
}

pub struct LocalUnlockSecret {
    bytes: Zeroizing<[u8; 32]>,
}

impl LocalUnlockSecret {
    pub fn generate() -> Self {
        let mut bytes = [0_u8; 32];
        rand::rng().fill_bytes(&mut bytes);
        Self { bytes: Zeroizing::new(bytes) }
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self { bytes: Zeroizing::new(bytes) }
    }

    pub fn to_storage_string(&self) -> String {
        STANDARD_NO_PAD.encode(self.bytes.as_slice())
    }

    pub fn from_storage_string(encoded: &str) -> Result<Self, LocalSecretError> {
        let decoded = STANDARD_NO_PAD.decode(encoded)?;
        if decoded.len() != 32 {
            return Err(LocalSecretError::InvalidSecretLength(decoded.len()));
        }

        let mut bytes = [0_u8; 32];
        bytes.copy_from_slice(&decoded);
        Ok(Self::from_bytes(bytes))
    }

    pub fn derive_sdk_store_key(&self) -> Result<[u8; 32], LocalSecretError> {
        derive_key(&self.bytes[..], b"matrix-desktop:sdk-store")
    }

    pub fn derive_search_key(&self) -> Result<Zeroizing<String>, LocalSecretError> {
        let key = derive_key(&self.bytes[..], b"matrix-desktop:search-index")?;
        Ok(Zeroizing::new(STANDARD_NO_PAD.encode(key)))
    }
}

impl Drop for LocalUnlockSecret {
    fn drop(&mut self) {
        self.bytes.zeroize();
    }
}

fn derive_key(input: &[u8], info: &[u8]) -> Result<[u8; 32], LocalSecretError> {
    let hk = Hkdf::<Sha256>::new(None, input);
    let mut out = [0_u8; 32];
    hk.expand(info, &mut out).map_err(|_| LocalSecretError::Derivation)?;
    Ok(out)
}

pub struct CredentialStore {
    service_name: String,
}

impl CredentialStore {
    pub fn new(service_name: impl Into<String>) -> Self {
        Self { service_name: service_name.into() }
    }

    pub fn save(&self, id: &SessionKeyId, secret: &LocalUnlockSecret) -> Result<(), LocalSecretError> {
        let entry = Entry::new(&self.service_name, &id.account_name())?;
        entry.set_password(&secret.to_storage_string())?;
        Ok(())
    }

    pub fn load(&self, id: &SessionKeyId) -> Result<LocalUnlockSecret, LocalSecretError> {
        let entry = Entry::new(&self.service_name, &id.account_name())?;
        let encoded = entry.get_password()?;
        LocalUnlockSecret::from_storage_string(&encoded)
    }

    pub fn delete(&self, id: &SessionKeyId) -> Result<(), LocalSecretError> {
        let entry = Entry::new(&self.service_name, &id.account_name())?;
        entry.delete_credential()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespaced_keys_are_distinct() {
        let secret = LocalUnlockSecret::from_bytes([7_u8; 32]);

        let sdk = secret.derive_sdk_store_key().expect("sdk key");
        let search = secret.derive_search_key().expect("search key");

        assert_ne!(STANDARD_NO_PAD.encode(sdk), *search);
    }

    #[test]
    fn account_name_includes_homeserver_user_and_device() {
        let id = SessionKeyId {
            homeserver: "https://matrix.example.org".to_owned(),
            user_id: "@user-a:example.invalid".to_owned(),
            device_id: "DEVICE".to_owned(),
        };

        assert_eq!(
            id.account_name(),
            "https://matrix.example.org|@user-a:example.invalid|DEVICE"
        );
    }

    #[test]
    #[ignore = "uses the host OS credential store"]
    fn credential_store_round_trip() {
        let id = SessionKeyId {
            homeserver: "https://matrix.example.org".to_owned(),
            user_id: "@user-a:example.invalid".to_owned(),
            device_id: "DEVICE".to_owned(),
        };
        let store = CredentialStore::new("matrix-desktop-spike");
        let secret = LocalUnlockSecret::from_bytes([9_u8; 32]);

        store.save(&id, &secret).expect("save");
        let loaded = store.load(&id).expect("load");
        assert_eq!(loaded.to_storage_string(), secret.to_storage_string());
        store.delete(&id).expect("delete");
    }
}
```

- [ ] **Step 3: Run key-management tests**

Run:

```bash
cargo test -p key-management
```

Expected: PASS with one ignored credential-store test.

- [ ] **Step 4: Run live credential-store test on macOS**

Run on macOS:

```bash
cargo test -p key-management credential_store_round_trip -- --ignored --nocapture
```

Expected: PASS and the credential is deleted at the end.

- [ ] **Step 5: Update key-management spike record**

Append to `docs/spikes/key-management.md`:

```markdown
## Result

- Local unlock secret is 32 random bytes.
- SDK store key is a 32-byte raw key suitable for `SqliteStoreConfig::key`.
- Search key is a namespaced string suitable for `SearchIndexStoreKind::EncryptedDirectory`.
- Credential-store records are named by homeserver, user ID, and device ID.
- Missing credential handling must fail closed and offer local-state reset.
```

- [ ] **Step 6: Commit key-management spike**

Run:

```bash
git add spikes/key-management docs/spikes/key-management.md Cargo.toml
git commit -m "Add key management spike"
```

Expected: commit succeeds.

## Task 6: Final Spike Review Gate

**Files:**
- Create: `docs/spikes/2026-06-11-spike-results.md`
- Modify: `docs/superpowers/specs/2026-06-11-matrix-desktop-design.md`

- [ ] **Step 1: Run all spike verification commands**

Run:

```bash
cargo test -p sidebar-composition
cargo test -p key-management
cargo test --manifest-path vendor/matrix-rust-sdk/crates/matrix-sdk-search/Cargo.toml ngram -- --nocapture
cargo test --manifest-path vendor/matrix-rust-sdk/crates/matrix-sdk/Cargo.toml search_index --features experimental-search -- --nocapture
```

Expected: all commands PASS.

- [ ] **Step 2: Write spike results summary**

Create `docs/spikes/2026-06-11-spike-results.md`:

```markdown
# Prerequisite Spike Results

Date: 2026-06-11

## Search

Result: pass

Evidence:
- `cargo test --manifest-path vendor/matrix-rust-sdk/crates/matrix-sdk-search/Cargo.toml ngram -- --nocapture`
- `cargo test --manifest-path vendor/matrix-rust-sdk/crates/matrix-sdk/Cargo.toml search_index --features experimental-search -- --nocapture`

Remaining implementation work:
- full late-decryption reindex path;
- snippet/highlight generation for UI;
- full index rebuild UI and progress reporting.

## Sidebar Composition

Result: pass

Evidence:
- `cargo test -p sidebar-composition`

Remaining implementation work:
- replace spike inputs with SDK stream adapters;
- add nested Space and multi-parent room UI decisions.

## Key Management

Result: pass

Evidence:
- `cargo test -p key-management`

Remaining implementation work:
- run ignored credential-store test on Windows;
- integrate `SqliteStoreConfig::key` in the Tauri backend;
- define logout and missing-secret UI.
```

- [ ] **Step 3: Update spec status**

In `docs/superpowers/specs/2026-06-11-matrix-desktop-design.md`, change:

```markdown
Status: Draft for written spec review
```

to:

```markdown
Status: Spike-validated; ready for full implementation planning
```

- [ ] **Step 4: Commit final spike gate**

Run:

```bash
git add docs/spikes/2026-06-11-spike-results.md docs/superpowers/specs/2026-06-11-matrix-desktop-design.md
git commit -m "Record prerequisite spike results"
```

Expected: commit succeeds.

## Execution Order

Run tasks in this order:

1. Task 1 creates the reproducible SDK and spike workspace.
2. Task 2 patches `matrix-sdk-search` tokenizer support.
3. Task 3 proves search event lifecycle behavior.
4. Task 4 proves desktop sidebar composition independently.
5. Task 5 proves key-management independently.
6. Task 6 records evidence and unlocks the full app implementation plan.

Tasks 4 and 5 can run in parallel after Task 1. Tasks 2 and 3 are sequential because Task 3 depends on the search config patch from Task 2.
