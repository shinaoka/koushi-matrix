# Search Adapter Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a pure Rust search adapter foundation that verifies Matrix search candidates before the UI sees snippets or highlights.

**Architecture:** Add a `matrix-desktop-search` crate that depends on `matrix-desktop-state` but not on Tauri or Matrix SDK. The crate owns local resolved-search document state, pending edit handling, redaction cleanup, exact span verification, UTF-16 highlight range generation, and conversion into `SearchResult` DTOs. Future backend code will map Matrix SDK event-cache events and `matrix-sdk-search` event-id candidates into this crate.

**Tech Stack:** Rust 2024, serde, `matrix-desktop-state`, table-driven tests, TDD.

---

## Scope

This plan does not wire Tauri, React, live Matrix SDK clients, or the encrypted Tantivy index. It creates the deterministic adapter layer that lets those pieces stay small later:

1. A redacted sensitive string wrapper for decrypted searchable text.
2. A local document store that treats edit-before-target events as pending.
3. Redaction cleanup for base events and pending edits.
4. Exact candidate verification over resolved visible body or attachment filename.
5. UTF-16 highlight ranges suitable for the frontend.

## Planned File Structure

```text
matrix-desktop/
  Cargo.toml
  crates/
    matrix-desktop-search/
      Cargo.toml
      src/
        document.rs
        lib.rs
        sensitive.rs
        verify.rs
      tests/
        search_adapter.rs
  docs/
    superpowers/plans/
      2026-06-11-search-adapter-foundation.md
```

`matrix-desktop-search` is intentionally SDK-free. It must not open files, call keyrings, start network work, or own a long-lived SDK client.

## Task 1: Scaffold the Search Adapter Crate

**Files:**
- Modify: `Cargo.toml`
- Create: `crates/matrix-desktop-search/Cargo.toml`
- Create: `crates/matrix-desktop-search/src/lib.rs`

- [ ] **Step 1: Add a failing crate import test**

Create `crates/matrix-desktop-search/tests/search_adapter.rs`:

```rust
use matrix_desktop_search::SearchDocumentStore;

#[test]
fn search_document_store_can_be_created() {
    let store = SearchDocumentStore::default();

    assert_eq!(store.document_count(), 0);
}
```

- [ ] **Step 2: Run the test to verify RED**

Run:

```bash
cargo test -p matrix-desktop-search search_document_store_can_be_created
```

Expected: FAIL because the package or `SearchDocumentStore` does not exist.

- [ ] **Step 3: Add the crate to the workspace**

Modify root `Cargo.toml`:

```toml
[workspace]
members = [
    "crates/matrix-desktop-state",
    "crates/matrix-desktop-search",
    "spikes/sidebar-composition",
    "spikes/key-management",
]
resolver = "2"
```

Create `crates/matrix-desktop-search/Cargo.toml`:

```toml
[package]
name = "matrix-desktop-search"
version = "0.1.0"
edition = "2024"
license = "MIT"

[dependencies]
matrix-desktop-state = { path = "../matrix-desktop-state" }
serde = { version = "1", features = ["derive"] }
thiserror = "2"
```

Create `crates/matrix-desktop-search/src/lib.rs`:

```rust
mod document;
mod sensitive;
mod verify;

pub use document::{SearchDocumentStore, SearchableEvent, SearchCandidate, SearchEdit};
pub use sensitive::SensitiveString;
pub use verify::{SearchVerificationError, verify_candidate};
```

- [ ] **Step 4: Add the minimal store implementation**

Create `crates/matrix-desktop-search/src/document.rs`:

```rust
use std::collections::BTreeMap;

#[derive(Default)]
pub struct SearchDocumentStore {
    documents: BTreeMap<String, ()>,
}

impl SearchDocumentStore {
    pub fn document_count(&self) -> usize {
        self.documents.len()
    }
}

pub struct SearchableEvent;
pub struct SearchCandidate;
pub struct SearchEdit;
```

Create `crates/matrix-desktop-search/src/sensitive.rs`:

```rust
#[derive(Clone, Eq, PartialEq)]
pub struct SensitiveString(String);
```

Create `crates/matrix-desktop-search/src/verify.rs`:

```rust
use thiserror::Error;

#[derive(Debug, Error, Eq, PartialEq)]
pub enum SearchVerificationError {
    #[error("candidate event is not available")]
    MissingCandidate,
}

pub fn verify_candidate() -> Result<(), SearchVerificationError> {
    Ok(())
}
```

- [ ] **Step 5: Run the test to verify GREEN**

Run:

```bash
cargo test -p matrix-desktop-search search_document_store_can_be_created
```

Expected: PASS.

- [ ] **Step 6: Commit**

Run:

```bash
git add Cargo.toml crates/matrix-desktop-search docs/superpowers/plans/2026-06-11-search-adapter-foundation.md
git commit -m "Add search adapter crate scaffold"
```

Expected: commit succeeds.

## Task 2: Add Redacted Sensitive Text Types

**Files:**
- Modify: `crates/matrix-desktop-search/src/sensitive.rs`
- Modify: `crates/matrix-desktop-search/src/document.rs`
- Test: `crates/matrix-desktop-search/tests/search_adapter.rs`

- [ ] **Step 1: Write the failing redaction test**

Append to `crates/matrix-desktop-search/tests/search_adapter.rs`:

```rust
use matrix_desktop_search::{SearchableEvent, SensitiveString};

#[test]
fn debug_output_redacts_decrypted_search_text() {
    let event = SearchableEvent {
        room_id: "!room:example.org".into(),
        event_id: "$event".into(),
        sender: "@alice:example.org".into(),
        timestamp_ms: 1_700_000_000_000,
        body: Some(SensitiveString::new("secret body")),
        attachment_filename: Some(SensitiveString::new("secret.pdf")),
    };

    let debug = format!("{event:?}");

    assert!(!debug.contains("secret body"));
    assert!(!debug.contains("secret.pdf"));
    assert!(debug.contains("SensitiveString(..)"));
}
```

- [ ] **Step 2: Run the test to verify RED**

Run:

```bash
cargo test -p matrix-desktop-search debug_output_redacts_decrypted_search_text
```

Expected: FAIL because `SearchableEvent` does not have those fields and `SensitiveString::new` does not exist.

- [ ] **Step 3: Implement redacted sensitive text and event DTO**

Replace `crates/matrix-desktop-search/src/sensitive.rs`:

```rust
use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct SensitiveString(String);

impl SensitiveString {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for SensitiveString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SensitiveString(..)")
    }
}
```

Replace the placeholder event structs in `crates/matrix-desktop-search/src/document.rs` with:

```rust
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::SensitiveString;

#[derive(Default)]
pub struct SearchDocumentStore {
    documents: BTreeMap<String, SearchableEvent>,
}

impl SearchDocumentStore {
    pub fn document_count(&self) -> usize {
        self.documents.len()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchableEvent {
    pub room_id: String,
    pub event_id: String,
    pub sender: String,
    pub timestamp_ms: u64,
    pub body: Option<SensitiveString>,
    pub attachment_filename: Option<SensitiveString>,
}

pub struct SearchCandidate;
pub struct SearchEdit;
```

- [ ] **Step 4: Run the test to verify GREEN**

Run:

```bash
cargo test -p matrix-desktop-search debug_output_redacts_decrypted_search_text
```

Expected: PASS.

- [ ] **Step 5: Commit**

Run:

```bash
git add crates/matrix-desktop-search
git commit -m "Add redacted search text types"
```

Expected: commit succeeds.

## Task 3: Verify Exact Message Body And Filename Matches

**Files:**
- Modify: `crates/matrix-desktop-search/src/document.rs`
- Modify: `crates/matrix-desktop-search/src/verify.rs`
- Test: `crates/matrix-desktop-search/tests/search_adapter.rs`

- [ ] **Step 1: Write failing exact verification tests**

Append to `crates/matrix-desktop-search/tests/search_adapter.rs`:

```rust
use matrix_desktop_state::{SearchMatchField, SearchMatchKind, TextRange};
use matrix_desktop_search::SearchCandidate;

#[test]
fn exact_message_body_match_returns_utf16_highlight() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(SearchableEvent {
        room_id: "!room:example.org".into(),
        event_id: "$event".into(),
        sender: "@alice:example.org".into(),
        timestamp_ms: 1_700_000_000_000,
        body: Some(SensitiveString::new("再アンケートです")),
        attachment_filename: None,
    });

    let result = store
        .verify_candidate(
            SearchCandidate { room_id: "!room:example.org".into(), event_id: "$event".into(), score_millis: 900 },
            "アンケート",
        )
        .expect("candidate should verify");

    assert_eq!(result.event_id, "$event");
    assert_eq!(result.snippet, "再アンケートです");
    assert_eq!(result.match_field, SearchMatchField::MessageBody);
    assert_eq!(result.match_kind, SearchMatchKind::Exact);
    assert_eq!(result.highlights, vec![TextRange { start_utf16: 1, end_utf16: 6 }]);
}

#[test]
fn attachment_filename_match_uses_attachment_field() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(SearchableEvent {
        room_id: "!room:example.org".into(),
        event_id: "$file".into(),
        sender: "@alice:example.org".into(),
        timestamp_ms: 1_700_000_000_000,
        body: None,
        attachment_filename: Some(SensitiveString::new("seminar_schedule.pdf")),
    });

    let result = store
        .verify_candidate(
            SearchCandidate { room_id: "!room:example.org".into(), event_id: "$file".into(), score_millis: 875 },
            "schedule",
        )
        .expect("filename candidate should verify");

    assert_eq!(result.event_id, "$file");
    assert_eq!(result.snippet, "seminar_schedule.pdf");
    assert_eq!(result.match_field, SearchMatchField::AttachmentFileName);
    assert_eq!(result.highlights, vec![TextRange { start_utf16: 8, end_utf16: 16 }]);
}

#[test]
fn ngram_false_positive_without_exact_span_is_dropped() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(SearchableEvent {
        room_id: "!room:example.org".into(),
        event_id: "$event".into(),
        sender: "@alice:example.org".into(),
        timestamp_ms: 1_700_000_000_000,
        body: Some(SensitiveString::new("再アンケートです")),
        attachment_filename: None,
    });

    let result = store.verify_candidate(
        SearchCandidate { room_id: "!room:example.org".into(), event_id: "$event".into(), score_millis: 900 },
        "欠席",
    );

    assert!(result.is_none());
}
```

- [ ] **Step 2: Run the tests to verify RED**

Run:

```bash
cargo test -p matrix-desktop-search --test search_adapter
```

Expected: FAIL because `upsert_message`, `verify_candidate`, and `SearchCandidate` fields are missing.

- [ ] **Step 3: Implement exact candidate verification**

Update `crates/matrix-desktop-search/src/document.rs` to define `SearchCandidate`, `upsert_message`, and `verify_candidate`. Update `crates/matrix-desktop-search/src/verify.rs` to find the first exact byte span, convert it to UTF-16 offsets, and produce a `SearchResult`. Body matches take precedence over filename matches.

Required behavior:
- Return `None` if the event is missing.
- Return `None` if the query is empty.
- Return `None` if neither visible body nor visible filename contains the query exactly.
- Use `SearchMatchKind::Exact`.
- Use candidate `score_millis` unchanged.

- [ ] **Step 4: Run the tests to verify GREEN**

Run:

```bash
cargo test -p matrix-desktop-search --test search_adapter
```

Expected: PASS.

- [ ] **Step 5: Commit**

Run:

```bash
git add crates/matrix-desktop-search
git commit -m "Add exact search candidate verification"
```

Expected: commit succeeds.

## Task 4: Handle Edit-Before-Target And Redaction

**Files:**
- Modify: `crates/matrix-desktop-search/src/document.rs`
- Test: `crates/matrix-desktop-search/tests/search_adapter.rs`

- [ ] **Step 1: Write failing edit and redaction tests**

Append to `crates/matrix-desktop-search/tests/search_adapter.rs`:

```rust
use matrix_desktop_search::SearchEdit;

#[test]
fn edit_before_target_is_pending_until_original_arrives() {
    let mut store = SearchDocumentStore::default();
    store.upsert_edit(SearchEdit {
        edit_event_id: "$edit".into(),
        target_event_id: "$original".into(),
        sender: "@alice:example.org".into(),
        timestamp_ms: 1_700_000_000_100,
        body: Some(SensitiveString::new("edited agenda")),
        attachment_filename: None,
    });

    assert_eq!(store.pending_edit_count(), 1);
    assert!(store.verify_candidate(
        SearchCandidate { room_id: "!room:example.org".into(), event_id: "$original".into(), score_millis: 900 },
        "edited",
    ).is_none());

    store.upsert_message(SearchableEvent {
        room_id: "!room:example.org".into(),
        event_id: "$original".into(),
        sender: "@alice:example.org".into(),
        timestamp_ms: 1_700_000_000_000,
        body: Some(SensitiveString::new("old agenda")),
        attachment_filename: None,
    });

    let result = store.verify_candidate(
        SearchCandidate { room_id: "!room:example.org".into(), event_id: "$original".into(), score_millis: 900 },
        "edited",
    ).expect("pending edit should apply after original arrives");

    assert_eq!(store.pending_edit_count(), 0);
    assert_eq!(result.event_id, "$original");
    assert_eq!(result.snippet, "edited agenda");
}

#[test]
fn redacted_event_is_not_returned() {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(SearchableEvent {
        room_id: "!room:example.org".into(),
        event_id: "$event".into(),
        sender: "@alice:example.org".into(),
        timestamp_ms: 1_700_000_000_000,
        body: Some(SensitiveString::new("visible before redaction")),
        attachment_filename: Some(SensitiveString::new("visible.pdf")),
    });

    store.redact("$event");

    assert_eq!(store.document_count(), 0);
    assert!(store.verify_candidate(
        SearchCandidate { room_id: "!room:example.org".into(), event_id: "$event".into(), score_millis: 900 },
        "visible",
    ).is_none());
}
```

- [ ] **Step 2: Run the tests to verify RED**

Run:

```bash
cargo test -p matrix-desktop-search --test search_adapter
```

Expected: FAIL because edit and redaction APIs are missing.

- [ ] **Step 3: Implement pending edit and redaction state**

Update `SearchDocumentStore`:
- Store base events by original event ID.
- Store latest applied edit by target event ID.
- Store pending edits by target event ID when the target is missing.
- On message upsert, apply the latest pending edit for that event and clear pending edits for that target.
- On edit upsert, apply immediately if target exists, otherwise keep pending.
- On redaction, remove the base event, applied edit, and pending edits for the redacted ID.

When multiple edits target the same event, choose the greatest `(timestamp_ms, edit_event_id)` tuple as the latest edit.

- [ ] **Step 4: Run the tests to verify GREEN**

Run:

```bash
cargo test -p matrix-desktop-search --test search_adapter
```

Expected: PASS.

- [ ] **Step 5: Run all crate tests**

Run:

```bash
cargo test -p matrix-desktop-search
```

Expected: PASS.

- [ ] **Step 6: Commit**

Run:

```bash
git add crates/matrix-desktop-search
git commit -m "Handle pending edits and redactions in search adapter"
```

Expected: commit succeeds.

## Task 5: Document Adapter Contract

**Files:**
- Create: `docs/architecture/search-adapter.md`

- [ ] **Step 1: Add the architecture doc**

Create `docs/architecture/search-adapter.md`:

```markdown
# Search Adapter

The search adapter verifies local search candidates before the UI receives any
snippet or highlight.

`matrix-sdk-search` owns encrypted Tantivy indexes and returns candidate event
IDs. `matrix-desktop-search` owns deterministic verification over resolved
visible event content. The Tauri backend will map Matrix SDK event-cache updates
into the adapter and map verified adapter results into reducer actions.

The adapter does not store data on disk, open OS secrets, call the network, or
own a Matrix SDK client. It keeps only the in-memory resolved document snapshot
needed to verify current search candidates.

Security rules:

- decrypted text is wrapped in redacted debug types;
- ngram candidates are dropped unless an exact visible span is found;
- highlight ranges are UTF-16 offsets relative to the returned snippet;
- attachment filenames use `SearchMatchField::AttachmentFileName`;
- edit events downloaded before their targets are held as pending relations;
- redactions remove base events, applied edits, and pending edits.
```

- [ ] **Step 2: Run verification**

Run:

```bash
cargo test -p matrix-desktop-search
cargo test -p matrix-desktop-state
```

Expected: both PASS.

- [ ] **Step 3: Commit**

Run:

```bash
git add docs/architecture/search-adapter.md
git commit -m "Document search adapter contract"
```

Expected: commit succeeds.

## Self-Review

- Spec coverage: covers remaining spike work for snippet/highlight generation, pending edits, redaction handling, and safe attachment filename search. Encrypted index opening remains in `matrix-sdk-search` and key integration remains a later Tauri backend task.
- Placeholder scan: no TBD/TODO placeholders.
- Type consistency: `SearchDocumentStore`, `SearchableEvent`, `SearchEdit`, `SearchCandidate`, `SensitiveString`, and `SearchResult` are introduced before use.
