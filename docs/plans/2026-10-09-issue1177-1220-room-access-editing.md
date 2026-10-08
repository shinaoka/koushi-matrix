# #1177 + #1220: room access and history — choice-and-detail editing and verified Space-member labels

Status: design revision 2. Revision 1 was reviewed by an independent GPT-6.1 Sol
review (`docs/reviews/2026-10-09-issue1177-1220-design-review.md`) which returned
three blockers and eleven other items; all are folded in below and the review
record stays in the branch. No implementation started. Worktree
`/tmp/koushi-access`, branch `feat/1177-1220-room-access-editing`.

## Scope

One deliverable: the same reusable choice-and-detail interface for join
conditions and history visibility in **Create Room** and **Room Info**, owned by
Rust, plus the people-facing access label that reports it.

- #1177: "Members of selected Spaces" selectable and restorable with real
  membership allow conditions; `private` never presented as that route; the four
  history policies through the same interface; the combined outcome visible;
  selection separate from commitment.
- #1220: the room-list badge, the in-room header badge and the Room Info access
  summary say **Space members can join** only for the verified single-Space allow
  route, and keep the generic **Conditions apply** otherwise; Room Info stops
  summarising `restricted`/`knock_restricted` as `Private`; no invented names and
  no leaked IDs.

Non-goals: no change to the permission model beyond reusing the existing facts
correctly, no new SDK fork, no round-trip machinery for unmodelled condition
types, no ordinary-room allow-target selection in the editor, no rededesign of
the Space access section.

## Verified current state (corrections from review round 1 included)

- `RoomSettingsSnapshot` (`crates/koushi-state/src/state/room_management.rs`)
  carries `join_rule`, `history_visibility` and `permissions` only; no directory
  visibility, no allow-condition detail.
- `RoomJoinRule::is_settable` admits `Public | Invite | Knock | Private`; the
  settable-rule restriction is also canon
  (`docs/architecture/state-machine.md:3758-3763`) and must be amended here.
- `RoomSettingChange::JoinRule(RoomJoinRule)` cannot carry an allow list.
- The restricted-condition projection loses information:
  `matrix_room_restricted_conditions`, `matrix_room_restricted_allow_room_ids`
  and `RoomAccessCondition` return/keep only the membership entries and report
  `Usable` as soon as one membership entry exists, so "one Space membership" and
  "that membership plus an unsupported condition" project identically. The badge
  and the editor both depend on that distinction.
- Target type and target name are different facts, and the current name path is
  weak: a missing cached display name falls back to the room ID, and that fallback
  is carried into `access_allowed_room_names`. `is_space`/`room_type` /
  `create_content` availability are the authoritative signals
  (`vendor/matrix-rust-sdk/crates/matrix-sdk-base/src/room/mod.rs`); a `None`
  room type is not positive proof of an ordinary room.
- The production join-rule write is an ordinary
  `privacy_settings().update_join_rule(JoinRule)` state event
  (`crates/koushi-sdk/src/room_operations.rs`), which already accepts full
  restricted content.
- Creation **already** sends a real membership allow condition for a standard
  private room in a Space (`crates/koushi-sdk/src/room_operations.rs`, the create
  preset path, room version V9). What creation cannot express is an
  *independently selected* policy and allow list; `visibility`, `invitedOnly` and
  `parentSpace` are the only inputs today
  (`apps/desktop/src/App.tsx` `createRoomRequestFromDraft`,
  `crates/koushi-protocol/src/command/room.rs`,
  `crates/koushi-core/src/room/operations.rs`), and the concrete presets are
  derived in the SDK adapter, not in Core.
- The in-room header is `apps/desktop/src/components/panes.tsx`
  (`sidebarRoomAccess`, header badges and tooltips). Both the header and the row
  tooltip special-case the existing generic label ID; changing the label without
  those branches loses the Space name from the tooltip. Room Info has its own
  binary Public/Private implementation, no access tooltip, and suppresses access
  summaries for DMs. `knockRestricted` currently carries two routes — conditions
  and request — and the new label must replace only the membership-route badge,
  keeping "Can request".
- Permissions: `can_edit_settings` is a conjunction over name/topic/avatar/join
  rules/history, while `can_change_join_rule` is event-specific and is what
  `RoomPermissionFacts::allows_setting_change` uses for join rules. Room Info
  currently gates the join rule on the aggregate flag, so an account allowed to
  change join rules but not to rename the room is blocked by the renderer
  although Core would authorize the command. `SpaceAccessSection.tsx` already
  uses the correct flag.
- Live coherence today: `RoomAccessUpdated` updates only the access map,
  successful setting changes update only `room_management.settings`, and the
  open-settings reconciliation searches Spaces only and compares only the
  join-rule enum — so an ordinary room's settings, and a change from
  `restricted(A)` to `restricted(B)`, are not reconciled.
- Directory publication is not projected anywhere; the SDK exposes
  `get_room_visibility` (`vendor/matrix-rust-sdk/.../room/privacy_settings.rs`).

## Upstream comparison (corrected)

`REPOSITORY_RULES.md` requires Element Web **and** Element X Android/iOS.

- **Element X Android** has restricted-policy editing:
  `features/securityandprivacy/impl/.../SecurityAndPrivacyView.kt` (Space-member
  and combined request/member choices) with `SecurityAndPrivacyPresenter.kt`
  (authorised-Space selection, mapping to `JoinRule.Restricted` /
  `KnockRestricted` with membership conditions), and creation supports the same
  variants (`features/createroom/.../ConfigureRoomPresenter.kt`, `JoinRuleItem.kt`).
- **Element X iOS** has the same flow:
  `SecurityAndPrivacyScreenViewModel.swift` (authorised-Space selection, existing
  non-parent targets, restricted and knock-restricted write shapes) and
  `View/SecurityAndPrivacyScreen.swift` (visible choices with descriptions).
- **Element Web** exposes the two policies as separate groups in the room
  security tab — **Access** (join rule) and **Who can read history?** — each a
  select, with a playwright spec that exercises differing permissions between
  them. (Revision 1 claimed there is no allow-list editor upstream; that claim is
  not evidenced and is withdrawn.)
- **Intentional divergences**, recorded: (a) per-property Save with a draft
  preview versus the mobile screens' screen-level Save; (b) four distinct history
  values versus the mobile fold of joined/invited; (c) directory publication
  preserved and disclosed rather than adjusted on save as Android does; (d)
  stricter verified target-type and unsupported-condition handling than the
  mobile mappings, which filter to membership entries.

## Rust-owned contract

Rust owns effective policy, validation, allow conditions, the outcome vocabulary
and every authoritative fact; React owns the uncommitted selection and rendering.

1. **Allow-condition completeness.** Retain, from the room's own join-rule
   content, an independent fact distinguishing confirmed-empty, membership-only,
   membership-plus-unsupported, and unsupported-only. The editor offers the
   membership editor only for the membership-only case; a mixed or
   unsupported-only rule is shown as such and an edit that would rewrite it is
   rejected with a typed failure and **no state-event write**. No round-trip
   machinery for unmodelled condition types.
2. **Target classification and naming, kept separate.**
   `AllowTarget { kind, display_name }` where `kind` is verified from the local
   room's create event (`Space`, `Room`, `Unknown` where the create event is
   unavailable or redacted), and `display_name` is optional and only populated
   from a name that is safe to show (never a room ID). Count all distinct targets
   **before** dropping unnamed ones, and keep unnamed identities inside Rust when
   rejecting or preserving an edit.
3. **One pure outcome resolver.** A pure Rust function over
   (join-rule policy, history policy, encryption, directory visibility, target
   facts) returning the message-catalog IDs and substitutions for "who can
   join", "who can read which history", "encrypted", "listed in the published
   directory". It is used by (a) the confirmed room settings projection and
   (b) a stateless preview entry point modelled on the existing room-address
   preview (`apps/desktop/src-tauri/src/commands/room.rs`,
   `crates/koushi-state/src/room_address.rs`). No new persistent outcome slice.
   The preview contract states explicitly which policies it describes: the
   property's own draft combined with the other property's **confirmed** value,
   and the panel shows that distinction, because Save commits per property.
4. **Settable restricted rules.** `RoomSettingChange` gains an explicit
   access-policy override carrying the rule plus the selected allow Space IDs;
   a restricted rule is settable only when the override carries a verified
   allow list, and `private` stays reserved and is never offered as the Space
   route. The canonical comparison of "the current policy" is the full policy
   value (rule + allow targets), so switching allow Spaces is a change even
   though the rule enum is unchanged.
5. **Creation.** The create request gains an optional explicit access-policy
   override (rule + allow Space IDs) and an explicit history policy, with these
   rules: the UI always submits an explicit policy; when the override is omitted,
   the existing legacy fallback (the private-in-Space preset, including its V9
   pin) is preserved verbatim; `visibility`/`invitedOnly` keep their current
   public-room stripping; a membership selection at Home is not allowed (the
   choice states why); the selected allow Space must be an authorised Space and
   need not be the attachment Space; an omitted allow list and an explicitly empty
   list are different values; history defaults are resolved once and applied as
   one override so only one initial-state write is sent. Choosing a Space as an
   allow target never requires permission on that Space and never implies writing
   `m.space.child`; the existing attachment/link failure report stays separate
   from creation success.
6. **Directory publication.** Read it with the SDK's `get_room_visibility` when
   the room settings load, scoped to the queried directory, and project
   confirmed / unavailable / loading / failed distinctly. Changing a join rule
   never changes publication implicitly.
7. **Permissions.** `RoomPermissionFacts::allows_setting_change` stays the single
   admission authority. The join-rule control renders from `can_change_join_rule`
   (fixing the current mismatch); the history control keeps the existing aggregate
   permission, and that divergence from Element is documented rather than
   silently upgraded.
8. **Live coherence.** Extend the existing access observation and
   reconciliation path to complete policy facts instead of adding a second
   subscription owner: reconcile ordinary rooms too, compare the full policy
   value rather than the enum, and keep working while an older SDK value is still
   cached until the local write echoes. A change by another client (rule, allow
   targets, or history) refreshes open settings without a reload or a Space
   change.
9. **Failure vocabulary.** Invalid or unsupported policy edits get a typed,
   specific failure rather than the generic SDK failure, and the renderer shows
   it on the property. No new general property-operation framework.

## React contract

- One shared component (working name `AccessChoiceDetail`) used by Room Info's
  access/history section and by `CreateEntityDialog`, replacing the two
  `InlineChoicePropertyEditor` instances used for these policies and the create
  dialog's visibility/`invitedOnly` controls. It reuses the existing
  `SettingsPropertyCard` status/save affordances rather than a second status
  framework.
- Left: every relevant choice with a short label and a one-line summary, the
  selected one marked. Right: the details for the selection, built from the Rust
  outcome for the draft-combined-with-confirmed pair, stating whether it
  describes confirmed or unsaved values.
- Room Info commits on Save and creation on Create; Cancel restores the
  confirmed value; pending/saved/failed/read-only stay on the property.
- Narrow and short windows stack list and details, keeping every choice and the
  actions reachable; keyboard navigation, visible focus, accessible
  selection/detail association, EN/JA and pseudo-locale all hold.
- Unavailable choices state the concrete reason (room version, permission,
  encryption) instead of being hidden.
- The three access surfaces consume one Rust-projected indicator contract
  (specific/generic fact plus tooltip substitutions): the room-list row, the
  in-room header in `panes.tsx`, and the room summary. The row and header tooltip
  branches that special-case the current generic label ID are updated together,
  the `knockRestricted` request route keeps its own badge, the summary keeps its
  navigation to the property, Room Info stops rendering restricted rules as
  `Private` and stops special-casing DMs, and the summary keeps showing the
  confirmed value while the editor holds a draft.

## Acceptance criteria

Rust / state:

1. Confirmed-empty, membership-only, membership-plus-unsupported and
   unsupported-only rules are four distinct projections; a rejected edit
   produces no state-event write.
2. A verified single-Space route yields the specific sentence and the safe Space
   name; an ordinary-room target, several targets, unknown type, unavailable name
   or no usable condition yields the generic sentence with no raw ID; `private`
   is never the Space route.
3. Setting restricted with a verified allow list round-trips; without one it is
   rejected; the canonical policy comparison treats a changed allow list as a
   change.
4. Creating in a Space with the membership choice produces a server room whose
   join rule carries that Space; an explicit history policy is sent; omitting
   both reproduces today's presets; two parent attachments with one explicitly
   selected allow Space is honoured as selected.
5. `can_change_join_rule = true, can_edit_settings = false` submits a join-rule
   change successfully, and a forbidden direct command is still rejected.
6. Another client changing only the allow targets, and another changing history,
   converge in open settings without reload or Space change.
7. Directory publication is reported as confirmed/unavailable/loading/failed and
   is unchanged by a join-rule edit.

Frontend:

8. Both surfaces render the same structure: all choices reachable, selection
   distinct from the confirmed value, details following the selection, Save
   disabled on a real change including an allow-list-only change, per-surface
   commit semantics, per-property status.
9. The three access surfaces agree for one room in Home, a Space, People/DM,
   favourites and low-priority contexts; the tooltip names the Space on hover and
   keyboard focus; both `knockRestricted` badges survive.

Browser-headless / production path:

10. The browser payload submitted from each surface is asserted, not only the
    standalone component.
11. `qa:headless-local` on both `--server=tuwunel` and `--server=synapse`:
    create with the membership choice and read back the synced allow content and
    history visibility; restore a restricted rule through the real update command
    and read back the synced allow content; observe a second-client allow-target
    change without refresh.
12. Narrow **and short** viewports with Japanese, English, accented and bidi
    pseudo-locales keep every choice and the actions reachable (visibility alone
    is not enough).

Each behaviour records the RED check first (command, exit status, captured
failure) and then the same check GREEN.

## Contract surfaces to mirror

`crates/koushi-state/src/state/room_management.rs` (snapshot, change enum,
policy comparison, outcome), `crates/koushi-state/src/sidebar.rs` and the room
access projection (indicator facts), `crates/koushi-core/src/room/{operations.rs,
management.rs,list_observer.rs}` and the reconciliation path,
`crates/koushi-sdk/src/{room_operations.rs,room_projection.rs}` (write adapter and
success projection — the current conversion rejects restricted rules),
`crates/koushi-protocol/src/command/room.rs` and
`crates/koushi-protocol/src/event/room.rs`,
`apps/desktop/src-tauri/src/dto.rs` plus the preview command registration,
`apps/desktop/src/domain/{types.ts,accessCondition.ts,coreEvents.generated.json}`,
`apps/desktop/src/components/{RoomInfoPanel.tsx,dialogs.tsx,panes.tsx,Shell.tsx,SettingsPropertyCard.tsx}`,
the test fixtures and `frontend_app_state` golden, the forwarded state-delta
fixture (`apps/desktop/src-tauri/src/core_event_forwarder/tests.rs`), the canon
(`docs/architecture/state-machine.md`, including the settable-rule restriction),
and the user guide.

## Verification plan (iteration)

Focused `cargo test -p koushi-state -p koushi-core -p koushi-protocol -p koushi-sdk`,
`cargo fmt --check`, clippy `-D warnings` for changed crates, golden regenerate +
normal run, `npm run typecheck`, `npm run lint`, focused `npx vitest run`, the
touched Playwright specs from `apps/desktop`. Landing gate before the PR: the
feature-unified workspace suite plus `qa:headless-local -- --server=both` per
`docs/policies/engineering-rules.md`.

## Delete list (applied from the review)

- No independent persistent outcome slice: extend existing access/sidebar facts
  and share one pure resolver with the draft preview.
- No new Matrix write mechanism or SDK fork: `update_join_rule(JoinRule)` already
  carries restricted content.
- No arbitrary unsupported-condition round-trip: reject the affected edit with
  the completeness fact.
- No ordinary-room allow-target selection in this deliverable: preserve or reject
  the existing ones.
- No speculative creation modes or capability framework: expose only the agreed
  routes with concrete reasons.
- No second property-card/status/focus framework: reuse `SettingsPropertyCard`.
- No duplicate Tauri settings DTOs or mechanical command-registration changes.

## Open questions for the reviewer

1. Whether creation should expose `knock_restricted` now or defer it with a
   stated reason (the private-in-Space preset pins room version V9; a combined
   request/member route needs its supported-version answer).
2. Whether the history control should get its own event-specific permission fact
   in this deliverable or keep the aggregate permission with a documented
   divergence.
