# MCP operations independent of GUI state (#1524)

Correctness follow-up to [#1520](2026-10-18-issue1520-mcp-bridge.md). The
mandatory contract is in the issue: an MCP call must not change the selected
account or room, panes, GUI search state, composer drafts, focus, scroll or read
state, and its progress, errors and cancellation belong to the calling MCP
client alone. Domain changes that the normal sync path projects — a sent message
appearing in an open timeline — stay allowed.

## Where the #1520 bridge violates it

| Tool | Current path | Violation |
| --- | --- | --- |
| `search_messages` | `SearchCommand::Query` | Reduces `AppAction::SearchSubmitted`, so the GUI search pane query/results/loading change. One `active_search_request`; the `SearchActor` coalesces pending queries and aborts the previous SDK query, so a GUI search can cancel an MCP search and two MCP callers can cancel each other. The runtime drops the caller's `room_filter`, so a room-restricted query can return other rooms. |
| `get_attachment` | `TimelineCommand::DownloadMedia` | Routed through `TimelineManagerActor`; an unviewed room has no actor (`NotSubscribed`) and the actor needs the event in its `media_sources` cache. Retrieval therefore depends on presentation lifetime. |
| `send_message` | `TimelineCommand::SendText`/`SendReply` | Required a subscribed timeline enqueue context. Fixed in `2bd9c639`: Core resolves the joined room and uses the SDK room queue, and direct sends never submit or settle a composer. |
| `list_accounts`, `list_rooms`, `wait_for_events` | runtime descriptors, snapshots, event log | No selected-room dependency found; still needs matrix coverage. |

## Design rule

MCP operations run as **in-process Core calls that never reduce app actions and
never emit an event the Tauri adapter forwards to the WebView**. They reuse the
single account session, encrypted index, media policy and send queue. No new
crypto store, background service, or Matrix session is introduced.

This is enforced by construction rather than by filtering: the direct calls
below return their outcome on a per-request reply channel, so nothing enters the
`CoreEvent` broadcast or the reducer.

One bilateral exception was added later, in the rule text of
`docs/architecture/overview.md`: `send_thread_reply` (#1853) may open the thread
pane when the thread belongs to the room the main pane displays, and only in the
account tab the app displays. It reduces the same `OpenThread` action the
renderer uses, never changes the account, the selected Space or the selected
room, and reports what it did — opened, or the reason it did not — in the reply
outcome instead of in an event.

### Direct search

`CoreConnection::direct_search(account_id, query, room_ids, limit)` sends an
internal `DirectRequest` envelope — not a protocol DTO — to the account runtime
with a `oneshot` reply, and returns `DirectSearchOutcome`:

- executes the same index search and candidate verification path as the GUI
  search (`run_sdk_query`), with the caller's `room_ids` filter;
- validates every requested room against that account's joined membership and
  fails closed otherwise;
- owns its own request id, task and lifetime: the GUI's latest-query behavior
  cannot cancel it, and independent MCP calls cannot cancel each other;
- bounded concurrency per account (four); exceeding the bound returns a typed
  capacity failure;
- dropping the future (caller timeout or shutdown) aborts the SDK task and frees
  the slot;
- never touches `active_search_request`, `AppAction::SearchSubmitted`, or
  `AppState.search`.

### Direct attachment

`CoreConnection::fetch_attachment(account_id, room_id, event_id, thumbnail)`
resolves the room and event from the account session directly — no timeline
actor, no presentation subscription, no cached `media_sources` entry:

- enforces joined membership and event existence at the authoritative boundary;
- downloads and decrypts through the existing media policy and SDK session;
- writes into the managed cache with the existing containment check and returns
  path, mime type and size;
- fails closed on missing keys or lost access.

### Adapter

`mcp/handlers.rs` calls the two direct methods instead of `SearchCommand::Query`
and `TimelineCommand::DownloadMedia`. Grants, account validation and argument
validation stay in the adapter.

## Slices

1. Display-independent send — done (`2bd9c639`), verified by the focused
   outbound suite and the full Core library suite.
2. Direct search in Core plus adapter switch — done. `search_messages` now
   calls `CoreConnection::direct_search`; the runtime forwards each request with
   the account's content policy, the account actor runs it under a four-permit
   bound with the caller's joined-room filter, and the reply is the caller's
   own. Tests: the GUI search state and owner survive an MCP search, an unjoined
   room fails closed, and a blank query answers empty without an SDK call.
3. Direct attachment in Core plus adapter switch — done. `get_attachment` no
   longer emits a timeline command; `koushi_core::timeline::fetch_attachment`
   resolves the joined room and event from the session, reuses the GUI download
   and managed-cache path, and the adapter re-checks containment. Tests: an
   unjoined room fails closed, a failed fetch writes nothing, and the runtime
   publishes no GUI event and changes no navigation state.
4. Isolation matrix, architecture canon and user help, integrated review, PR.

## Verification

Behavioral tests in `koushi-core` with synthetic fixtures and the real SDK mock
server where the path calls the SDK; no live accounts.

- search: GUI search state (query, results, loading, selected room) is identical
  before, during and after an MCP search; two overlapping MCP searches each get
  their own results; an in-flight GUI search is not cancelled; a room filter
  returns only that room; capacity and cancellation produce typed terminal
  outcomes and free slots.
- attachment: retrieval from an unviewed room and from an event outside the
  rendered window; encrypted media through the existing session; navigation and
  unmount during retrieval do not cancel it; containment holds; missing key and
  access loss fail closed.
- send: the existing unviewed-room tests plus draft/selection preservation on
  acceptance, success, failure and cancellation.
- matrix: no selected room, a different selected room or account, two account
  runtimes, concurrent callers, and missing/locked/removed/replaced sessions.
