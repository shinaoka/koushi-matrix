# #1177 + #1220: room access and history — choice-and-detail editing and verified Space-member labels

Status: design revision 1, awaiting an independent pre-implementation review. No
implementation started. Baseline `origin/main` (after #1223) = `fbdb7211`.
Worktree `/tmp/koushi-access`, branch `feat/1177-1220-room-access-editing`.

## Scope

One deliverable: the same reusable choice-and-detail interface for join
conditions and history visibility in **Create Room** and **Room Info**, owned by
Rust, plus the people-facing access label that reports it.

- #1177: "Members of selected Spaces" must be selectable and restorable with real
  membership allow conditions; `private` must never be presented as the way to
  let Space members join; the four history policies must be exposed through the
  same interface; the combined outcome (who can join, who can read which
  history, whether messages are encrypted, whether the room is listed) must be
  visible; selection must be separate from commitment.
- #1220: when a room's own `m.room.join_rules` is `restricted` or
  `knock_restricted` and the sole verified usable allow target is a Space, the
  room-list badge, the in-room header badge and the Room Info access summary say
  **Space members can join** / **スペース参加者は参加可** with a full tooltip naming
  the Space; otherwise they keep the generic **Conditions apply** / **参加条件有**.
  Room Info must stop summarising `restricted`/`knock_restricted` as
  `Private`/`非公開`. Unsupported or unknown allow data never becomes a specific
  claim and never leaks a raw ID.

Non-goals: no change to who may edit what (the existing permission model), no new
Matrix API beyond the SDK's existing join-rules/delayed-state calls, no
redesign of the Space access section (#1166 shipped), no directory-publication
feature work beyond disclosing the current value.

## Verified current state

- `RoomSettingsSnapshot` (`crates/koushi-state/src/state/room_management.rs:90`)
  carries `join_rule: RoomJoinRule` and `history_visibility`, and
  `RoomJoinRule::is_settable` (`:239`) admits only `Public | Invite | Knock |
  Private`, because "the others need content the command does not carry (a
  restricted allow list)".
- `RoomSettingChange` (`:321`) has `JoinRule(RoomJoinRule)` and
  `HistoryVisibility(..)` — neither can carry an allow list.
- `RoomAccessCondition { allowed_room_ids: Vec<String>, .. }` exists
  (`:276` region) and the sidebar projects its IDs to
  `access_allowed_room_names` (`crates/koushi-state/src/sidebar.rs:648-660`) —
  names only, no verified target type.
- `RoomInfoPanel.tsx:419-440` edits the join rule with
  `InlineChoicePropertyEditor<RoomJoinRule>` over
  `SETTABLE_JOIN_RULES = ["public","invite","knock","private"]`
  (`RoomInfoPanel.tsx:799-809`), appending the current rule when it is not
  settable so the control never silently lands elsewhere, and `:442-470` edits
  history visibility with a second `InlineChoicePropertyEditor` plus per-option
  notes. Both are dropdown-style, not the required interface.
- The create surface is `CreateEntityDialog` in
  `apps/desktop/src/components/dialogs.tsx:239` with
  `CreateRoomDialogOptions { aliasLocalpart, encrypted, invitedOnly, topic,
  visibility: "private" | "public" }` (`:226`) and `CreateRoomRequest`
  (`apps/desktop/src/App.tsx:426-443`): Core derives the join rule and history
  from `visibility` + `invitedOnly` + `parentSpace`, so creation currently cannot
  express either a membership allow list or an explicit history policy.
- `SpaceAccessSection.tsx` already renders Space join rules including
  `restricted` (`accessModeMessage`, `:222-260`) — the vocabulary exists, the
  room-side editor does not.

## Upstream comparison (recorded before implementation)

- **Element Web** room settings expose the two policies as separate groups in the
  Security tab: **Access** (join rule) and **Who can read history?**, each a
  select, with a playwright spec that deliberately exercises differing
  permissions between them (`apps/web/playwright/e2e/settings/room-settings/room-security-tab.spec.ts`).
  `restricted` is offered as "Space members". There is no combined-outcome
  preview and no allow-list editor beyond the parent-Space option.
- **Element X Android** has no restricted join-rule editing surface.
- **Intentional divergence:** #1177 explicitly replaces the plain-dropdown design
  for these two policies with a visible choice list plus a details panel, and
  adds the combined outcome. We keep Element's permission separation (each policy
  stays independently editable per the existing permission facts) and keep the
  option vocabulary aligned with Element's labels.

## Rust-owned contract

Rust owns effective access policy, validation, membership allow conditions and
the typed outcome model; React owns the uncommitted selection and rendering.

1. **Allow-target classification.** Project each allow target of a room's own
   join rule as `AllowTarget { room_id, kind: AllowTargetKind, display_name:
   Option<String> }` with `AllowTargetKind { Space, Room, Unknown }`. `kind` comes
   from authoritative room type/ID information (the local room cache's
   `m.room.create` type through the SDK), never from a name, the parent Space,
   the viewer's membership, or a Space's own invite-only setting. Do not leak
   `room_id` to the renderer when the name is unavailable: use `Unknown` and the
   generic copy.
2. **Settable restricted rules.** Extend the join-rule change to carry the allow
   list, e.g. `RoomSettingChange::JoinRule { rule: RoomJoinRule, allowed_space_ids:
   Vec<String> }`, and make a restricted rule settable only when the change
   carries conditions. `private` stays a reserved Matrix value and is never
   offered as the Space-membership route. `Unknown` is displayed, never sent back.
3. **Creation.** Extend `CreateRoomRequest` / `CreateRoomDialogOptions` with an
   explicit history visibility and an optional membership allow list. Rust
   validates the referenced Spaces and the caller's permission, and derives the
   existing defaults when nothing is supplied (standalone private/public →
   `shared`; private room in a Space → `invited`) so current callers keep working.
   Attachment to a Space never silently becomes the allow list: the parent Space
   must be shown in the choice and explicitly included.
4. **Typed outcome.** Project `RoomAccessOutcome { who_can_join, who_can_read_history,
   encrypted, published_in_directory }` using message-catalog IDs plus
   substitutions, so React renders the combined result instead of deriving it.
   `who_can_join` reuses #1166/#1220 vocabulary: generic when multiple targets,
   unknown types, no usable condition or unresolved type; specific only for the
   verified single-Space case.
5. **Unsupported conditions are preserved.** A rule with condition types this
   client does not model stays visible as such, is never silently broadened, and
   an edit either round-trips them or is rejected with a typed failure.
6. **Live updates.** The room list, the timeline header and Room Info derive from
   the same projection and update together when the join rule changes (including a
   change made by another client), independent of the currently selected Space.

## React contract

- One shared component, e.g. `AccessChoiceDetail`, used by the Room Info
  access/history section and by `CreateEntityDialog`, replacing the two
  `InlineChoicePropertyEditor` used for these policies and the create dialog's
  visibility/`invitedOnly` controls.
- Left: all relevant choices with a short label and a one-line summary, the
  selected one clearly marked. Right: the details of the selected choice,
  including concrete outcomes, the configured Space membership conditions that
  affect them, encryption/key limits, directory publication and the interaction
  with the other policy, built from the Rust outcome DTO.
- Selection is a draft; Room Info commits only on Save and creation only on
  Create; Cancel restores the confirmed value; pending/saved/failed/read-only
  state stays on the property, not in a shared footer. The details panel states
  whether it describes the confirmed value or an unsaved draft.
- Narrow/short windows stack the list and the details without collapsing back
  into an unexplained dropdown; every choice and the Save/Create/Cancel actions
  stay reachable. Keyboard navigation, visible focus, accessible selection and
  detail association, EN/JA and pseudo-locale all hold.
- Choices that the room version, permissions, creation capabilities or
  encryption make unavailable are shown with the reason instead of being hidden.

## Acceptance criteria

Rust:

1. A `restricted`/`knock_restricted` room with one verified Space allow target
   projects the specific join sentence and the Space name; with an ordinary-room
   target, multiple targets, an unknown target type, an unavailable name or no
   usable condition it projects the generic sentence and never a raw ID.
2. `private` is never projected as the Space-membership route.
3. Setting a restricted rule with conditions succeeds and round-trips; setting one
   without conditions is rejected; an unmodelled condition type is preserved or
   the edit is rejected, never silently dropped.
4. Creating a room inside a Space with the membership choice produces a room whose
   server-side join rule carries that Space as an allow condition; creating with
   an explicit history policy sends it; omitting both keeps today's defaults.
5. Room Info no longer labels `restricted`/`knock_restricted` as `Private`, and
   the editor still shows the underlying rule.
6. A join-rule change by another client updates the room list, the header and
   Room Info together.

Frontend (vitest, DOM/aria):

7. Both surfaces render the same choice-and-detail structure: every choice
   reachable, selection distinct from the confirmed value, details following the
   selection, Save/Cancel semantics per surface, pending/failed/read-only on the
   property.
8. Unavailable choices state why; keyboard, focus and accessible associations
   hold; Japanese, English and pseudo-locale layouts do not clip or hide a choice.

Browser-headless (Playwright): narrow-window stacking keeps every choice and the
actions reachable, and the room-list badge, header badge and Room Info summary
show the same label for the same room.

Evidence discipline: each behaviour records the RED check first (command, exit
status, captured failure) and then the same check GREEN.

## Contract surfaces to mirror

`crates/koushi-state/src/state/room_management.rs` (settings snapshot, change
enum, outcome), `crates/koushi-core` (create-room and join-rule command paths),
`crates/koushi-protocol` (command DTOs), `apps/desktop/src-tauri/src/dto.rs` +
command registration, `apps/desktop/src/domain/types.ts`,
`apps/desktop/src/components/{RoomInfoPanel.tsx,dialogs.tsx,Shell.tsx}`, the
room-list/header badge helpers shipped by #1166, fixtures, the
`frontend_app_state` golden if the snapshot shape changes, and the user guide.

## Verification plan (iteration)

Focused `cargo test -p koushi-state -p koushi-core -p koushi-protocol`,
`cargo fmt --check`, clippy `-D warnings` for changed crates; golden regenerate +
normal run if the snapshot changes; `npm run typecheck`, `npm run lint`, focused
`npx vitest run`, the relevant Playwright specs from `apps/desktop`. The landing
gate before the PR is the feature-unified workspace suite plus
`qa:headless-local -- --server=both` per `docs/policies/engineering-rules.md`.

## Open questions for the reviewer

1. Whether the allow-list editor should offer Spaces only (recommended) or also
   allow an ordinary room target, given the issue says "Space members" but the
   Matrix rule permits room targets.
2. Whether creation should expose `knock_restricted` at all, or keep creation to
   the routes the create dialog can explain honestly.
3. Whether the outcome DTO should live on `RoomSettingsSnapshot` (room-scoped) or
   as its own slice so the room-list and header badges can share it without
   loading the settings surface.
