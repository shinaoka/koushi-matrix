# Always-On Unified Diagnostics Log

- Date: 2026-07-10
- Status: Design approved; written spec awaiting review
- Scope: `matrix-desktop` application diagnostics only

## Goal

Show every diagnostic signal intentionally added to the desktop application in
the Diagnostics dialog, regardless of diagnostic environment variables, while
preserving the repository's privacy rules and bounding memory use.

## Current State

Diagnostics are split across several unrelated paths:

- React keeps a bounded `DiagnosticLogEntry` list for timeline, panel, room
  transition, send, E2EE, and other GUI diagnostics.
- Uncaught JavaScript errors are captured separately and added to the report.
- Security diagnostics are hidden unless
  `VITE_KOUSHI_VERBOSE_DIAGNOSTICS=1`.
- Rust and Tauri diagnostic helpers write to stderr only when one of several
  environment variables is present. The current groups include sync, search,
  startup, subscribe/timeline, timeline-item, unread, core-actor, and Tauri
  command diagnostics.
- `CoreEvent::IntentLifecycle` provides a dedicated telemetry-lane outcome but
  is not collected by the Diagnostics dialog.

Consequently, a normal application run can omit information that already exists
in the codebase, and the Diagnostics dialog cannot show the Rust/Tauri side of a
failure.

## Scope Boundary

"Every diagnostic signal" means every explicit, application-owned diagnostic
helper or structured telemetry event used by the desktop runtime:

- sync, search, startup, subscribe, timeline-item, unread, and core-actor
  diagnostics;
- Tauri command submission, routing, completion, and elapsed-time diagnostics;
- intent lifecycle outcomes;
- the existing React timeline, panel, room-transition, send, and E2EE entries;
- uncaught JavaScript errors and current security diagnostics.

It does not mean arbitrary stdout/stderr from QA binaries, dependencies, the
operating system, or the Matrix SDK. Capturing arbitrary process output would
make the privacy boundary unverifiable.

## Considered Approaches

### 1. Shared bounded collector, fetched by the dialog (selected)

All application-owned Rust/Tauri diagnostic helpers record into a shared,
process-local ring buffer. Opening Diagnostics fetches a snapshot and merges it
with the frontend-owned entries.

This retains startup history, provides one privacy boundary, does not mix
telemetry with product state, and cannot block a product command on a WebView
consumer.

### 2. Stream every record to the WebView

A dedicated Tauri event could deliver diagnostics live. This is useful for a
live viewer but can miss records emitted before listener installation and adds
high-volume event traffic to the WebView. A history buffer would still be
needed, so streaming alone does not solve the requirement.

### 3. Capture stderr

Redirecting process stderr would require few call-site changes, but it cannot
prove that dependency output, raw SDK errors, identifiers, paths, or message
content are safe. It is rejected.

## Selected Architecture

### Shared collector

Add a leaf workspace crate at `crates/koushi-diagnostics`. It has no dependency
on the SDK, core, Tauri, or frontend crates, so `koushi-sdk`, `koushi-core`, and
`koushi-desktop` can all record into it without creating dependency cycles. It
owns a thread-safe, process-local ring buffer with these properties:

- maximum of 10,000 records;
- oldest record discarded when full;
- cumulative dropped-record count retained and shown in the report;
- snapshot reads clone only the current bounded records;
- a poisoned lock or clock failure cannot fail a product operation.

The collector is a telemetry facility only. It is not persisted, does not enter
`AppState`, and is never used to determine product success.

### Structured and private-data-free records

Rust/Tauri records use structured fields rather than accepting arbitrary log
strings. The common envelope contains:

- timestamp;
- severity;
- source/category;
- stage and operation kind when applicable;
- numeric request id when applicable;
- typed values such as booleans, counts, durations, and fixed outcome kinds.

There is no general-purpose raw-text field for Rust/Tauri producers. Matrix
room, user, event, and transaction identifiers; message/search text;
attachment names; local paths; URLs from rooms; raw SDK errors; and secrets are
prohibited. Existing traces that currently mention an identifier must be
represented by a fixed kind, presence boolean, count, or other explicitly
privacy-reviewed token before they enter the collector.

The frontend keeps its existing sanitization for JavaScript errors and legacy
GUI entries. Report formatting happens only after the Rust/Tauri snapshot and
frontend records have crossed their respective privacy filters.

### Environment variables

Diagnostic environment variables no longer decide whether a record is
collected. Application-owned diagnostics are always recorded.

Existing environment variables may continue to control a mirrored stderr line
for local QA scripts that parse those tokens. Thus an unset variable suppresses
console noise but never removes the corresponding record from Diagnostics.
Call sites that currently skip the diagnostic computation entirely must be
split into always-on structured capture and optional stderr mirroring.

`VITE_KOUSHI_VERBOSE_DIAGNOSTICS` is removed from the Diagnostics data path.
Security diagnostics are always included in the dialog.

### Tauri and frontend integration

Expose a read-only Tauri command that returns the bounded Rust/Tauri snapshot
and dropped-record count. Add the corresponding method to the desktop API
contract and browser fake.

When the user opens Diagnostics:

1. fetch the Rust/Tauri diagnostic snapshot;
2. retain the latest successful snapshot in React state;
3. merge it with frontend diagnostic entries by timestamp;
4. render the existing report sections plus one unified chronological log;
5. report fetch failure as a coarse diagnostic kind without preventing the
   dialog from opening.

The first implementation is snapshot-based, not live-streaming. Reopening the
dialog refreshes the snapshot. A future live viewer may add a dedicated event
stream while keeping this buffer as the source of startup history.

## Migration Inventory

The implementation must inventory and migrate all application runtime trace
families, including the current environment-gated helpers for:

- `KOUSHI_SYNC_TRACE`;
- `KOUSHI_SEARCH_TRACE`;
- `KOUSHI_STARTUP_TRACE`;
- `KOUSHI_SUBSCRIBE_TRACE`;
- `KOUSHI_TIMELINE_ITEM_TRACE`;
- `KOUSHI_UNREAD_TRACE`;
- `KOUSHI_CORE_ACTOR_TRACE`;
- the Tauri search and timeline command traces.

The inventory excludes QA-only binary progress/error printing. Each migrated
family needs a focused test proving environment-independent collection and
private-data-free formatting.

## Error Handling and Performance

- Recording is synchronous, bounded, and contains no I/O.
- Diagnostics collection must never await channel capacity or WebView work.
- Formatting and DTO conversion occur when a snapshot is requested, not on
  every product-state render.
- Expensive observations previously hidden behind an environment variable are
  measured during implementation. If an observation materially changes product
  latency, its always-on form must record an equivalent cheaper structured
  signal rather than omit the diagnostic family.
- A Diagnostics fetch error is represented by a coarse error kind; raw errors
  are not copied into the report.

## Verification Strategy

Implementation follows test-driven development:

1. Add failing collector tests for ordering, the 10,000-record bound, dropped
   count, and concurrent recording.
2. Add failing tests showing representative core and Tauri diagnostic helpers
   record with all related environment variables unset.
3. Add privacy tests with synthetic Matrix identifiers, paths, and message text
   and prove they cannot appear in serialized records.
4. Add a failing Tauri contract test for the read-only snapshot command.
5. Add a failing frontend test showing Diagnostics includes Rust/Tauri,
   frontend, JavaScript-error, security, and intent-lifecycle information with
   no verbose environment variable.
6. Run focused Rust and desktop tests, TypeScript type checking, formatting,
   and the repository's privacy/release gates.

Tests assert structured records and report content, not stderr output, except
for existing QA compatibility tests that explicitly cover stderr mirroring.

## Success Criteria

- A normal launch with no diagnostic environment variables produces a
  Diagnostics report containing all application-owned diagnostic families that
  emitted records during the retained window.
- Rust/Tauri and frontend records appear in chronological order.
- Security diagnostics are present without
  `VITE_KOUSHI_VERBOSE_DIAGNOSTICS`.
- The report states how many old records were discarded.
- No private Matrix data, content, local paths, secrets, or raw SDK errors can
  enter serialized diagnostics.
- Diagnostics collection cannot block or fail a product operation.

## Non-Goals

- Persistent logs across application restarts.
- Uploading diagnostics or telemetry.
- A new filterable/live log viewer UI.
- Capturing arbitrary dependency or operating-system output.
