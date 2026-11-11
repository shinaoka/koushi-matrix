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
now carries `thread_root_event_id`. The pane opens on that value; it never reads
the root again.

## How the pane opens

The room predicate and the pane live in the AppActor, so the caller's answer
waits there: `handle_direct_message_action` keeps a settled direct thread reply
in `pending_direct_thread_replies` (a `FuturesUnordered` of boxed oneshots) and
the run loop settles it. `handle_direct_thread_reply_settled` opens the pane with
the same `open_thread_pane` helper the renderer's `AppCommand::OpenThread` arm
uses (reduce `AppAction::OpenThread`, release the pane it replaces, dispatch the
effects), publishes the state change, and only then answers the caller with
`thread_pane_opened`. The renderer reveals the panel from that Rust state through
the #1854 presentation effect (`threadPaneIdentityForActiveRoom`).

The predicate is `state.timeline.room_id == the reply's room` (the reducer
`handle_open_thread` enforces the same guard) plus the session revision the
request was made in. Nothing else moves: no account, Space or room selection, and
no reducer action or UI state is added. A reply to another room, a settlement
after the session changed, and a failed root read all answer without a pane.

## Fixing the reveal guard

The #1854 presentation effect revealed once per room/root identity and never
cleared that guard, so a thread the user had opened and closed stayed silent for
the next opener — including this one. The effect now clears the guard when the
pane is closed.

## Verification

- `crates/koushi-core/src/runtime/tests/mcp_thread_reply_reveal.rs` (the
  contributor's RED test plus six cases): the shown room opens; another room
  sends and reports `not_opened`; a root the runtime never held opens; the
  account, Space and room are unchanged; a failed root read fails and opens
  nothing; a settlement after the user switched rooms opens nothing; a reply to
  the thread the user already has open keeps that pane's composer and staged
  uploads.
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

The predicate is this account runtime's displayed room, as frozen in the issue.
An MCP call naming a *background* account tab whose retained state still shows
that room opens the pane in that (not displayed) tab and reports `opened`; no
signal currently tells a runtime whether its tab is the selected one, and adding
one is new state beyond this change's scope.

The reveal rides the #1854 presentation effect, which reveals a thread identity
once. A user who, while a thread is open, switches the right panel to pinned
messages or the thread list keeps that panel: the agent reply marks that thread
as open in Rust (`opened`) but does not override the user's own panel choice.
That is the same behavior #1854 shipped with for a repeated navigation to one
thread, and clearing the guard on close already covers the common case.
