# Scheduled Send Reliable Delivery Design

## Goal

Make local-fallback scheduled messages survive an unavailable timeline actor and retry when Matrix delivery cannot be confirmed.

## Scope

The server-delayed-events path remains unchanged. The local-fallback path is changed only after its scheduled time arrives.

## Design

Each scheduled item receives a random Matrix transaction ID at creation, making its persisted scheduled ID unique across app restarts. At the scheduled time, `AppActor` marks the local item as dispatching and asks `AccountActor` to submit its message directly to the room with a deterministic Matrix transaction ID derived from that persisted ID. This route does not need a visible or subscribed timeline actor. A successful Matrix response removes the scheduled item; an uncertain response retries with the same transaction ID so the homeserver deduplicates it.

If no session or room is available, or Matrix rejects the request, `AccountActor` keeps the item and moves its next attempt to a bounded retry delay. The transient dispatching marker is not persisted, so a process restart returns an in-flight item to the normal due-item scan.

The dispatch request carries the origin `SessionKeyId`. `AccountActor` ignores it when an account switch has made another session active, preventing an old account's reservation from being sent by the new account.

Locking or switching accounts clears the in-memory UI projection but must not overwrite the origin account's persisted schedule with an empty store. The local timer is stopped until a `Ready` session reloads that account's schedule.

## Error Handling

Queue-insertion failures still emit the existing core failure event. They also reset the transient dispatch marker and reschedule the item, preventing the timer from spinning on an already-due item and preventing data loss.

## Verification

Add a runtime regression test that uses the existing no-SDK-session harness: after the due time, the scheduled item must remain present with a later retry time. This fails on the current implementation because it deletes the item before routing the send.
