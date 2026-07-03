# Design: Invite Autocomplete and Space Scope (#187)

Status: approved in chat, amended 2026-07-03
Issue: https://github.com/shinaoka/koushi-matrix/issues/187

## Goal

Add Element-compatible invite target completion for room and space invites, and
add a Koushi-specific scope choice when inviting people to a room inside a
Space. The product contract is Rust-owned. React renders DTOs, owns only dialog
draft/focus/popover state, and dispatches typed commands.

## Element Baseline

Element Web routes room and private-space invites through `InviteDialog` with a
target room id. Its invite UX combines recent users, room/member context, user
directory search, exact Matrix user id handling, and selected-user pills before
calling `MultiInviter(...).invite(targetIds)`.

Element excludes or disables users that should not be invited, such as self,
already joined/invited users, banned users, and users outside a non-federated
room's server boundary. Public Spaces primarily expose a share-link dialog, with
an invite entry that opens the same room invite dialog.

Observed upstream references:

- `apps/web/src/components/views/dialogs/InviteDialog.tsx`
- `apps/web/src/RoomInvite.tsx`
- `apps/web/src/utils/space.tsx`
- `apps/web/src/components/views/spaces/SpacePublicShare.tsx`

Element Web does not appear to expose an explicit child-room selector for
"room only" versus "parent Space plus room". Koushi adds that selector because a
space-restricted child room otherwise has an ambiguous invite result.

## UX Contract

Room and Space invite buttons open the same invite dialog. The input supports
completion by:

- exact or partial Matrix user id, for example `@alice:example.test`;
- Koushi local user alias;
- Rust-projected display label and original display label;
- known profile, room member, DM, and mention search terms;
- homeserver user directory results when the SDK/backend can provide them.

Selected targets become pills. Pressing Enter or pasting a valid Matrix user id
creates a pill after Rust validation. Invalid input is represented as Rust-owned
validation state and rendered by React; React must not repair the product state
or synthesize a valid target.

The dialog accepts multiple targets. A selected target can carry warnings or
disabled reasons, but the final invite command includes only actionable targets.
Email and 3PID invite are out of scope for this issue.

## Space Scope

When the destination is a Space itself, the command targets the Space room. If a
target is already joined or invited to that Space, the operation returns an
informational per-target result with the UI message `既にスペースにいます`. That
result is not a hard failure.

When the destination is a child room of a Space, React renders the Rust-projected
scope choices:

- room only;
- parent Space plus room.

If there is an active parent Space, that parent is preferred. If multiple
parents exist and no active parent can be selected, Rust exposes either a parent
choice or a room-only default with an explicit reason. If parent-Space invite
permission is missing, the combined option is disabled with a reason, while
room-only remains available when room invite permission allows it.

For parent Space plus room, the Rust operation plans destinations per target:

1. invite to parent Space unless the target is already joined/invited;
2. invite to child room if allowed;
3. settle each destination independently.

If the parent Space step is skipped because the target is already in the Space,
the result records the same informational message, `既にスペースにいます`, and
continues to the room invite. This satisfies the user requirement that existing
Space membership must not block inviting to the room.

## Rust State And Commands

Add an invite workflow state under `AppState`, with separate query and operation
state:

- query input, request id, status, validation error, and candidate list;
- selected target ids or target DTOs;
- scope plan for the current destination;
- batch operation id, pending destinations, per-target/per-destination result,
  retryable failure kind, and informational skipped result kind.

The state machine is reducer-owned and documented in
`docs/architecture/state-machine.md` when implemented. Reducer guards must cover
ready-session checks, stale request ids, duplicate targets, unknown rooms,
permission-disabled scopes, and late results after dialog reset.

Core commands/events should be typed around intent rather than UI details:

- search or validate invite targets for a destination room;
- clear/reset invite state;
- execute an invite batch with selected target ids and a Rust-projected scope;
- emit candidate, validation, planned-scope, progress, success, skipped, and
  failure events.

Core owns SDK calls, profile/user-directory lookup, local alias/profile/member
matching, membership checks, and the final destination plan. `koushi-sdk` remains
the thin adapter for Matrix SDK primitives such as user lookup, user directory
search, and room invite.

## DTO And Frontend

Mirror every public state and event field through:

- Tauri DTOs;
- TypeScript domain types;
- `coreEvents.generated.json`;
- browser fake snapshots;
- Tauri IPC mocks and app harness snapshots;
- serialization contract tests.

React may own input draft text, focus, popover visibility, and button hover
state. React must not compute Matrix membership eligibility, alias precedence,
Space parent selection semantics, or invite success/failure interpretation.

The visible UI may show Matrix user ids and display labels because those are
the selected account-facing entities. Logs, tests, QA title tokens, PR evidence,
and issue evidence must stay private-data-free.

## Verification

Use test-first implementation.

Rust reducer/state tests:

- invite target query pending/succeeded/failed/stale transitions;
- Matrix user id parse and invalid-input state;
- alias/display-label/member/profile candidate ordering;
- self, already joined/invited, banned, and permission-denied filtering;
- room-only versus parent-Space-plus-room scope planning;
- already-in-Space skip result with `既にスペースにいます`;
- child-room invite continues after parent skip.

Core/SDK tests:

- user directory/profile lookup maps coarse private-data-free errors;
- batch invite results map destination success, skipped, retryable failure, and
  terminal failure;
- parent Space skip remains idempotent and does not call invite unnecessarily.

Browser/Vitest or Playwright tests:

- room invite dialog autocomplete, Enter/paste pill creation, invalid input;
- Space invite already-member informational message;
- child-room scope selector command payload;
- parent-Space-plus-room shows skip message and still dispatches room invite.

Local QA evidence, if added, must emit token-only checks such as:

- `invite_autocomplete=ok`;
- `space_invite_scope=ok`;
- `parent_space_invite_skip=ok`.

## Out Of Scope

- New identity-server or email/3PID invite support.
- Recursive invite to every child room in a Space.
- Treating Matrix room aliases (`#room:server`) as invite targets.
- E2EE trust/unknown identity warnings beyond existing trust state.
