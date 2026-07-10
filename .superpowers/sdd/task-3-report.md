# Task 3 report: always-on timeline diagnostics

## RED/GREEN evidence

The new collector assertions were added before the production changes and run
with `KOUSHI_SUBSCRIBE_TRACE`, `KOUSHI_TIMELINE_ITEM_TRACE`,
`KOUSHI_UNREAD_TRACE`, and `KOUSHI_STARTUP_TRACE` removed from the process
environment.

RED evidence:

- `timeline_diagnostic_helpers_collect_typed_records_without_trace_env` failed
  because `core.timeline/actor_finish` was absent.
- `unread_helpers_collect_typed_records_without_trace_env` failed because no
  typed `core.unread` record was present.
- `reaction_and_read_signal_collector_fields_are_typed_and_private` failed
  because no typed reaction/read record was present.

GREEN evidence:

- `unread_trace`: 5 passed, 0 failed.
- `reaction_and_read_signal_handlers_emit_private_latency_traces`: 1 passed,
  0 failed.
- `timeline_diagnostic_helpers_collect_typed_records_without_trace_env`: 1
  passed, 0 failed.
- Full `koushi-core --lib`: 414 passed, 0 failed.
- The requested `--lib timeline_trace` filter completed with 0 tests, 0
  failures; no test name currently contains that exact filter.
- `cargo fmt --all -- --check`: passed.
- `git diff --check`: passed.
- `npm --prefix apps/desktop run qa:secret-scan`: passed.

## Producer/source coverage

The timeline sources now collect into `core.timeline` for actor operations and
scans, manager routing, pagination, and link-preview work. Timeline item
batch/item/diff producers collect into `core.timeline_item`. Event-cache
batch/item/diff producers collect into `core.event_cache`, including relation
kind and timestamp-presence buckets without event or sender identifiers.

Unread room-list, activity, and mark-read producers collect into
`core.unread`. Structured collection is unconditional; the existing stderr
mirrors remain controlled by their existing environment variables and retain
their legacy formatting/conditions.

Timeline startup/build/subscribe/pagination timing now captures
`Instant` unconditionally. Timeline callers no longer use
`startup_trace::now_if_enabled`, and that compatibility API was removed.
The event-cache observer is installed independently of stderr settings so its
provenance and item records are not skipped in normal runs.

## Privacy audit

Collector fields are limited to fixed tokens, booleans, counts, durations,
timestamp-presence buckets, and numeric request IDs. The item/cache collector
paths never pass event IDs, sender IDs, room IDs, message bodies, URLs, paths,
raw errors, or hashes into `DiagnosticEvent`. Existing hashed/redacted values
remain confined to optional legacy stderr formatting. Tests serialize the
collected events and assert synthetic room IDs, event IDs, sender IDs, and
message text are absent.

No arbitrary `String` or `&str` diagnostic payload field was added; dynamic
stage/kind/reason values are mapped to a fixed token allowlist with `other` as
the fallback.

## Preservation comparison

The comparison source was the untouched dirty timeline reference at
`/Users/hiroshi/projects/Element-dev/matrix-desktop/crates/koushi-core/src/timeline.rs`.
Comparing it with base `2b7eef8` identified 139 unique non-empty pre-existing
added lines. All 139 lines remain present in the current worktree after the
Task 3 changes (`missing_from_current=0`). The latency functions/tokens
`trace_timeline_actor_operation`, `trace_timeline_actor_scan`,
`send_reaction`, `redact_reaction`, `send_read_receipt`, `set_fully_read`,
`target_scan`, and unconditional `Instant::now` capture remain present.

The pre-existing dirty changes in the three Tauri command files were not edited
or staged.
