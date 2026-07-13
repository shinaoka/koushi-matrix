# Focused Timeline Projection Acknowledgement Design

**Issue:** #242

**Status:** Approved direction; written specification pending review

## Problem

Focused navigation currently treats a command-side `CoreConnection` observing the target event as display success. That observer is not the WebView consumer. The command can therefore publish `main_timeline_anchor` while the matching `InitialItems` projection was rejected as stale or was never applied to the app-level timeline store. The resulting state is a selected room with an anchor and no rendered rows.

The fix must remove this false-success state. It must not add sleeps, fixed retries, Activity-only replay, live-room fallback, visibility heuristics, or transport-owned product state.

## Scope

This change introduces one Core-owned Focused navigation state machine for Activity Recent, Activity Unread, search results, and date jumps. It binds actor generation, projection delivery, WebView acceptance, store application, and anchor publication to one navigation owner.

Thread projections use the same generation-aware acceptance contract and are covered by regression tests. This change does not migrate unrelated live-room streaming to a new delivery architecture.

## Ownership and boundaries

- `koushi-core` owns navigation progress, projection identity, generation validation, and the decision to publish an anchor.
- The focused timeline actor owns the retained projection for its active generation and is the only source for reprojection.
- Tauri transports typed commands and events. It does not infer success or own retries.
- The app-level timeline store decides whether an `InitialItems` payload was actually applied and reports that result.
- React views request subscription/reprojection on mount but do not publish navigation success.

## Identity

Each pending Focused navigation has:

- an opaque `navigation_id` allocated by Core;
- the focused `TimelineKey`;
- the active actor `generation`;
- the target event identifier;
- a projection revision identifying the retained `InitialItems` snapshot.

An acknowledgement is valid only when every identity field matches the currently pending navigation and active actor generation. Identifiers from an older navigation, actor, projection, room, or event are rejected without mutating the anchor.

## State machine

The Core state machine has the following externally meaningful phases:

1. `Opening`: room selection and focused actor creation are in progress.
2. `Projecting`: the active actor has retained a projection for its generation and Core has issued it to display consumers.
3. `AwaitingApplication`: the WebView accepted the matching key/generation and is applying it to the app-level store.
4. `Applied`: Core accepted a typed acknowledgement proving the matching projection is present in the app-level store.
5. `Anchored`: the reducer publishes `main_timeline_anchor` for the same navigation owner.

`Anchored` is reachable only from `Applied`. Closing or replacing a focused context cancels the pending owner before the previous generation can progress. A stale acknowledgement is a no-op with an explicit typed outcome; it is never converted into success.

## Projection protocol

`InitialItems` carries optional projection acknowledgement metadata for projection-backed Focused and thread delivery. The app listener passes the payload through the canonical timeline-store reducer. That reducer returns both the next store and an application result:

- `Applied`: the exact key/generation/revision now owns the canonical item set;
- `RejectedStale`: another generation already owns the key;
- `Ignored`: the event is not acknowledgement-bearing.

Only `Applied` causes the frontend to submit `AcknowledgeTimelineProjection`. Core validates the lease against the active actor and pending navigation. A valid Focused acknowledgement advances to `Applied` and publishes the anchor through the normal reducer. Thread acknowledgement proves projection acceptance but does not create a main-timeline anchor.

The acknowledgement command is idempotent. Repeating the same valid acknowledgement returns the current completed state; it does not publish a second transition.

## Recovery

The actor retains the latest acknowledgement-bearing projection for its active generation until it is acknowledged, superseded, or unsubscribed. `EnsureSubscribed { replay_existing: true }` asks that actor to reproject the retained snapshot. This supplies the recovery trigger after WebView listener remount and after a lost delivery.

There is no timer or retry counter. If delivery is lost, the navigation remains pending and no anchor is exposed. The next consumer readiness/subscription reconciliation reprojects from the actor-owned state. A replacement generation cannot replay the old lease because generation fencing rejects it before broadcast.

Tauri forwarding errors are propagated to diagnostics rather than silently reported as successful emission, but Tauri does not decide whether the store applied the projection.

## Common navigation entry point

Activity and search commands call a shared Focused navigation helper rather than duplicating close/select/open/wait/anchor sequencing. Date navigation enters the same Core workflow after resolving its target context. The helper waits for the Core navigation completion associated with its request; it no longer scans timeline events through a command-only connection.

## Ordering guarantees

- A projection may arrive before the React screen transition. The app-level listener still applies it because the store is app-owned rather than view-owned.
- An anchor cannot be published before a matching application acknowledgement.
- An acknowledgement cannot cross actor generations or navigation owners.
- Reprojection preserves the original navigation identity and current actor generation.
- Bursts of unrelated Core events cannot satisfy or replace the pending navigation.

## Verification

Behavioral tests must exercise real reducers, Core event ordering, and the canonical store rather than source-string assertions or fake snapshots alone:

1. `InitialItems` before screen transition is stored and acknowledged.
2. remount plus `EnsureSubscribed` replays the retained projection.
3. stale actor generation cannot issue or acknowledge a projection.
4. anchor remains absent until store application acknowledgement.
5. Activity Recent normal DM uses the common workflow.
6. Activity Unread normal event uses the common workflow.
7. thread reply projection uses the same acceptance contract without setting a main anchor.
8. search result and date jump use the common Focused workflow.
9. a large unrelated-event burst preserves key, generation, revision, and navigation owner.
10. simulated first-delivery loss recovers through actor reprojection without a room switch.

The implementation also runs the existing Rust workspace, frontend unit/type/lint, IPC contract, Tauri boundary, and relevant headless local-server checks before merge.

## Upstream behavior

Element's timeline navigation advances its visible navigation state only after the requested timeline is loaded and the target is scrolled into view. Koushi's equivalent invariant is stricter at the process boundary: the Core anchor is published only after the WebView's canonical store confirms application of the matching projection.

