# Room Timeline Cache Repair Design

## Goal

Repair holes in a room timeline cache without treating cache deletion as a normal automatic path.

## Behavior

- When the app opens a known room event by event id, it performs a best-effort targeted fetch for that event before opening the anchored or focused timeline.
- The targeted fetch must not delete cache data. If the event is already present in the SDK room event cache, it does nothing.
- If the event is missing, the app fetches that single event from the homeserver through the SDK room API so the SDK stores it in the room event cache.
- If the fetch fails, navigation continues through the existing focused/anchored timeline path.
- A manual room timeline cache reset is available from Room info as a last-resort user action.

## Manual Reset

- The reset is scoped to the selected room's timeline event cache.
- It does not clear Matrix credentials, E2EE keys, drafts, room settings, search data, or server history.
- It requires explicit user confirmation from Room info.
- After reset, the selected room timeline is resubscribed so the UI gets a fresh room timeline window.

## Non-Goals

- No automatic room cache deletion.
- No account-wide cache reset.
- No recovery of arbitrary gaps without a known event id in this change.
