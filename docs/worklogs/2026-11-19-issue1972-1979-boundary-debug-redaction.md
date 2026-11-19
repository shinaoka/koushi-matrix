# Raw Matrix ids in boundary `Debug` (#1972, #1979)

`Debug` on a public boundary type is artifact-facing: whatever it prints can be
copied into a CI log, a QA artifact, or an issue comment. #1979 reported two such
sites; the sweep it asked for found the class around them.

## Sites

| Site | Leak |
| --- | --- |
| `koushi-state` `TimelineMediaGalleryItem` (#1972, PR #1976) | printed `event_id` raw while redacting every other field |
| `koushi-protocol` `AccountKey`, `TimelineKey`, `TimelineKind` | derived `Debug`, so 21 `Command` arms printed the account id and, through the kind, the room id or a thread's root event id |
| `TimelineCommand` | thirteen arms printed `key` by value, `EditText` also printed its `event_id` |
| `AppCommand` | 8 composer/focused-context arms printed `room_id` raw while their neighbours printed `RoomId(..)` |
| `RoomEvent::MarkedAsUnread` | printed `room_id` raw |
| `RoomCommand::InviteTargets` | redacted `room_id` but printed `scope`, whose `ParentSpaceAndRoom` names a real space |
| `SearchCommand::Query` | redacted `query` but printed `scope`, a `SearchScope` naming a real room or space (`Attachments`' scope was already redacted) |
| `ThreadsListCommand` | listed every resolved `room_ids` entry and printed `ThreadsListScope` raw |
| `ThreadsListEvent` | printed `room_id` and the whole `items` list: room id, root event id, senders, and both body previews per thread |
| `ThreadSummaryDto` | printed `latest_event_id`, `latest_sender`, `latest_body_preview` |
| `TimelineViewportObservation` | printed the first and last visible event ids |
| `RoomMessagesPage` | printed `next`, a live server page token |
| `TimelineUnableToDecrypt` | printed the megolm session id |
| `RoomLiveSignalMetadata` | printed the read marker and every typing sender |
| `SearchIndexMessage` (`koushi-core`) | printed room, event, edit, and target ids while redacting the body; `Edit` also omitted `room_id`/`sender` altogether, so both are now named and redacted |

## Shape

Two mechanisms, chosen by what the field is:

- **Types that redact themselves** for `AccountKey`, `TimelineKey`,
  `TimelineKind`, `SearchScope`, `ThreadsListScope`, `InviteScopeSelection`,
  `RoomLiveSignalMetadata`, `ThreadSummaryDto`, `TimelineViewportObservation`,
  `ThreadsListEvent`, `RoomMessagesPage`, `TimelineUnableToDecrypt`. A type cannot
  print an id, so an arm that prints the value is safe and a new arm cannot
  regress the rule by binding a field. `ThreadsListEvent` prints `items_len`
  instead of the items, and `RoomMessagesPage` `messages_len` plus whether a page
  token is present.
- **Per-field placeholders** where the field is a bare `String` (`room_id`,
  `event_id`, `room_ids`), because a `String` cannot redact itself. Those arms name
  the placeholder their neighbours already used, and the now-unused pattern
  binding becomes `room_id: _` or `event_id: _`.

An `Option` field prints `Some("EventId(..)")`/`None` rather than an unconditional
placeholder, so "absent" stays distinguishable from "present" — the same shape the
read-state impls in `event/timeline.rs` already use. `TimelineKey`'s `Debug` keeps
the literal `TimelineKey(..)` that the crate's 45 arms print.

`Serialize`, `Hash`, `PartialEq`, `Clone`, and `Copy` are untouched everywhere;
only `Debug` output moved.

## Evidence

Each new test failed before its fix. The first six (ids, `TimelineCommand`,
`AppCommand`, `SearchIndexMessage`) were run before the change was written; the
rest were run in a detached `git worktree` at the pre-change commit with only the
tests copied in. `ThreadsListEvent::Opened` rendered both body previews of every
thread, `RoomMessagesPage` rendered `next: Some("private-page-token")`, and
`EditText` rendered:

```
EditText { ... key: TimelineKey { account_key: AccountKey("@a:test"), kind: Room { room_id: "!room:test" } }, event_id: "$private-event:test", ... }
```

The tests assert the placeholder is present and that no `$…`, `!…`, or `@…`
identifier and no body text or page token appears.

## What is still open (#1985)

`CoreEvent::StateDelta` and the snapshot DTOs are still open paths:
`VersionedAppStateSnapshot` derives `Debug` over the whole `AppState`,
`StateDeltaChangedSlices` reaches `SessionInfo` (raw `homeserver`, `user_id`,
`device_id`), and `ThreadsListItem` prints its previews when formatted directly.
This change closes the protocol `Debug` impls and the DTOs they print, not the
`AppState` / `StateDelta` payload.

That class is deferred because `format!("{snapshot:?}")` is the app's general
diagnostic dump and tests prove secret absence *through* its verbosity (for example
`koushi-core`'s notification-delivery tests check that a send key is absent from the
rendered sink). Making the dump opaque would leave those assertions vacuous; the
alternative is to retarget them at the secret type and then redact the dump, which
is its own change.

Also left deliberately: the 45 arms that print `&"TimelineKey(..)"` now duplicate
what the type prints. Collapsing them means re-binding `key` in 45 patterns with no
change to the output, so it is a separate cleanup.
