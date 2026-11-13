# #1883 — MCP lists Spaces so same-named rooms resolve

## Contract

[Issue #1883](https://github.com/shinaoka/koushi-matrix/issues/1883): an
assistant posting to "room `all` in Space X" could not tell which `all` it was —
`list_rooms` carries no Space membership and no tool listed Spaces, so the agent
correctly stopped and asked the user. Spaces are first-class in the UI and were
invisible over MCP. Every addition here is read-only, needs `read` and nothing
else, and never changes the selected account, Space or room.

Canon consulted before code: `REPOSITORY_RULES.md`, `docs/architecture/overview.md`
"External MCP Bridge" (one owner per session, explicit account, no GUI change),
`crates/koushi-state/src/state/room.rs` (`SpaceSummary`, `RoomSummary`),
`docs/policies/engineering-rules.md` (test placement, no guessed values).

## What it answers, and from where

`list_spaces` and `list_space_rooms` are pure projections over the snapshot the
app already holds (`koushi-mcp-host/src/spaces.rs`), like `list_rooms` before
them: no new runtime, command, action or store.

- Parent Spaces come from the snapshot's own `SpaceSummary` lists — a Space that
  names the child in `child_room_ids` or in `parent_side_child_room_ids` — so the
  relation is found from both sides rather than from one possibly stale field.
- `list_space_rooms` walks `child_room_ids`, the display union the sidebar shows,
  breadth-first in room-id order (that list is a sorted set, not the sidebar's
  order) up to `max_depth` (1 default, 3 at most), keeps the first sighting of a
  repeated child, and starts `seen` with the requested Space so a self-listing or
  a cycle between two Spaces terminates.
- Nothing is invented where the snapshot knows nothing: `alias` is always `null`
  because Koushi keeps no canonical Space alias and never promotes one into a
  name; `suggested` is `null` unless the loaded child list belongs to that
  child's parent Space (`SpaceChildrenState::children_for` answers for one Space
  at a time, so an absent flag is unknown, not `false`); `is_space` is true only
  when the child is known to be a Space; and `name` is `null` for a child nothing
  names, including one whose only name is the SDK's calculated empty-room
  placeholder.
- A child this account has not joined is still typed and named when the loaded
  child list of its parent holds it, which is the same source and the same guard
  as `suggested`.
- Either reply carries at most 500 entries and says `truncated` when there were
  more: an unbounded Space listing could outgrow the bridge's 1 MiB frame and
  surface as `koushi_unavailable`. Narrowing `max_depth` reads the rest.
- An id that is not a Space in the snapshot is `room_not_found`, never an empty
  listing: an empty list would read as "this Space has no rooms".

`list_rooms` gained `parent_space_ids` per room and an optional `space_id`
filter over that same field, and keeps every existing field and default. The
filter deliberately reads the room's own `parent_space_ids` — the value the
reply carries — so what a caller sees is what filters. That field comes from the
SDK's own-side parent relation, while both Space tools read the parent's display
union, so a room added to a Space can appear in `list_space_rooms` and not yet in
`list_rooms` with that `space_id`; both the tool description and the user guide
say so instead of presenting the routes as interchangeable.

## Verification

RED first: the schema tests (`tools.len()`, required-argument list, read-only
flags), the registration manifest equality test, and both new bridge tests
failed before the handlers existed (`unsupported: unknown tool: list_spaces` /
`list_space_rooms`); the bridge tests cover the issue's own fixture — two rooms
displayed `all` in different Spaces plus a sub-Space — end-to-end through the
real socket, consent broker and reducer.

No new headless QA lane: the deterministic equivalent of the issue's Done-when
lives in `crates/koushi-mcp-host/tests/bridge/spaces.rs`, and Core's own Space
projection against disposable homeservers is already covered by the `room_space`
scenario in the nightly aggregate. Driving the bridge from the `koushi-qa`
binary would need a bridge host and consent surface there, which this change
does not add.

Gates run locally (macOS, arm64): `cargo fmt --check`;
`cargo clippy --profile ci -p koushi-mcp -p koushi-mcp-host --all-targets
--locked -- -D warnings`; `cargo test -p koushi-mcp --profile ci`;
`cargo test -p koushi-mcp-host --profile ci`; `node
scripts/check-rust-test-structure.mjs`; `node scripts/check-agents-docs.mjs`;
`node scripts/check-headless-qa-lanes.mjs`.

Independent review (`openai/gpt-6.1-sol`, read-only, on the finished diff)
raised three IMPORTANT points: a loaded unjoined sub-Space reported as
`is_space: false` and unnamed (fixed: the loaded child list now types and names
it); an unbounded listing against the 1 MiB frame (fixed: the 500-entry cap and
`truncated`); and a cycle back to the requested Space omitted from the listing
(dismissed: the reply's `space_id` already names it, and seeding `seen` with it
is what terminates a Space that lists itself). Two documentation points were
taken: `max_depth` clamping and the two-route sync difference. A second
read-only pass (`ollama-cloud/glm-5.3-flash`) independently raised the same
sync difference and clamping, and found no correctness or authorization
defect. A pre-merge review of the finished PR (batch 5) returned MERGE AFTER
FIX with one required correction, and it was a false ordering claim rather than
code: the child list is in room-id order (`child_room_ids` is built from a
`BTreeSet` and re-sorted on an optimistic link), not the sidebar's order, so
the tool description, the traversal comment and this worklog now say so.
