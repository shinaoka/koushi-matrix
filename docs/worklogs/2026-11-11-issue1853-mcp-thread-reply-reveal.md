# #1853 — MCP thread reply reveals the thread pane of the shown room

## Contract

[Issue #1853](https://github.com/shinaoka/koushi-matrix/issues/1853): an MCP
`send_thread_reply` accepted in the room the GUI is showing left the thread pane
closed, so the user could not see the reply. The owner froze design option 1 in
the issue: open the thread pane only when the thread belongs to the room the main
pane displays, on the root the send resolved, and never change the account, the
selected Space or the selected room. The RED test is the contributor's (#1855,
`6c5ae1ca`, authored by @hz-xiaxz), merged here with `cherry-pick -x`.

Canon consulted before code: `REPOSITORY_RULES.md` (canon-first, root cause,
ownership, security, test placement), `docs/architecture/overview.md` "External
MCP Bridge", `docs/architecture/state-machine.md` thread pane, the
[#1524 plan](../plans/2026-10-18-mcp-ui-isolation.md), and
`docs/policies/engineering-rules.md` (async/runtime, test placement).

Canon amended in this change: `docs/architecture/overview.md` (the #1524
exception clause: when an agent action may open a pane) and
`docs/architecture/state-machine.md` (which openers reduce `OpenThread`).

## Where the root comes from

`send_thread_reply` already reads its target once with the bounded authoritative
read (`timeline/direct_message.rs`, `load_event_authoritative`, `READ_TIMEOUT`)
before sending. That read resolves the thread root, so `DirectThreadReplyOutcome`
now carries `thread_root_event_id`. The reveal opens on that value and reads
nothing itself.

The thread pane's own cache-first root load still applies, measured rather than
assumed: `timeline/manager/thread_hydration_tests.rs` pins that opening a thread
whose root the room's loaded timeline already holds issues no event read, and
that a root outside it costs exactly one (the one the user opening the same
thread would pay). Removing that last read would need an SDK way to store the
resolved event in the room's event cache; `Room::load_or_fetch_event` is
cache-first and deliberately does not save, and no public `RoomEventCache` insert
exists, so it stays out of this change.

## How the pane opens

The room predicate and the pane live in the AppActor, so the caller's answer
waits there: `handle_direct_message_action` keeps a settled direct thread reply
in `pending_direct_thread_replies` (a `FuturesUnordered` of boxed oneshots) and
the run loop settles it. `handle_direct_thread_reply_settled` opens the pane with
the same `open_thread_pane` helper the renderer's `AppCommand::OpenThread` arm
uses (reduce `AppAction::OpenThread`, release the pane it replaces, dispatch the
effects), publishes the state change, and only then answers the caller with
`thread_pane`. The renderer reveals the panel from that Rust state through the
#1854 presentation effect (`threadPaneIdentityForActiveRoom`).

The predicate is: the session revision the request was made in is unchanged,
this account's tab is the one the app displays
(`AccountWorkScheduler::is_selected_account`, the scheduler's existing
selection, not a new signal), and `state.timeline.room_id` is the reply's room
(the reducer `handle_open_thread` enforces the same guard). A thread that is
already the open one is kept as it is, so its composer and staged uploads
survive; a `Failed` pane is retried instead of reported as open. Nothing else
moves: no account, Space or room selection, and no reducer action or UI state is
added. A reply to another room, through a background account tab, after the
session changed, or to a root that cannot be read all answer without a pane and
name why in `thread_pane`.

## Fixing the reveal guard

The #1854 presentation effect revealed once per room/root identity and never
cleared that guard, so a thread the user had opened and closed stayed silent for
the next opener — including this one. The effect now clears the guard when the
pane is closed.

## Verification

- `crates/koushi-core/src/runtime/tests/mcp_thread_reply_reveal.rs` (the
  contributor's RED test plus eight cases): the shown room opens; another room
  sends and reports `not_opened_other_room`; a root the runtime never held opens;
  the account, Space and room are unchanged; a failed root read fails and opens
  nothing; a settlement after the user switched rooms opens nothing; a reply to
  the thread the user already has open keeps that pane's composer and staged
  uploads; a `Failed` pane is retried; the selected account tab opens while a
  background tab leaves its pane closed, untouched, and says
  `not_opened_background_account`.
- `timeline/manager/thread_hydration_tests.rs` pins the root read count: none for
  a root the room cache holds, exactly one for a root it does not.
- `timeline/direct_message_tests.rs` asserts the resolved root on the real
  mock-server send path, including a target that is a reply inside the thread.
- `koushi-mcp` schema test pins `thread_pane`, `koushi-mcp-host` maps it.
- Gates, in the CI shapes: `cargo fmt --check`; `cargo clippy --profile ci
  --workspace --all-targets --locked -- -D warnings`; the QA-binary and
  `--release --workspace` clippy steps; the CI stack budget step `cargo clippy
  --release --workspace --locked -- -D warnings -D clippy::large_stack_frames
  -D clippy::large_futures` (production targets only, as `clippy.toml` states);
  `cargo test --workspace -j6 --no-fail-fast`; the Rust test-structure and
  leaf-crate checkers; the desktop typecheck, lint, and the full vitest suite;
  the user-help/llms check. Adding `--all-targets` to the stack-budget step
  fails on pre-existing test-harness futures, which that gate excludes.

## Known limits

The reveal rides the #1854 presentation effect, which reveals a thread identity
once. A user who, while a thread is open, switches the right panel to pinned
messages or the thread list keeps that panel: the agent reply marks that thread
as open in Rust (`opened`) but does not override the user's own panel choice.
That is the same behavior #1854 shipped with for a repeated navigation to one
thread, and clearing the guard on close already covers the common case.

The account predicate is the scheduler's selected tab, which the app sets on
selection, not an observation of the rendered window. A second window or a
tab whose selection is unknown to the manager is outside it; the reveal treats
an unknown selection as selected, because such a runtime is the only account
there is (headless QA, unit tests).
