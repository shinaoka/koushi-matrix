# #1177 + #1220: room access and history — choice-and-detail editing and verified Space-member labels

Status: design revision 3. Two independent GPT-6.1 Sol reviews are recorded in
`docs/reviews/2026-10-09-issue1177-1220-design-review.md` (round 1: three
blockers) and this file's predecessor (round 2: five resolved, eight partially
resolved, three new items). Revision 3 folds both in. No implementation started.
Worktree `/tmp/koushi-access`, branch `feat/1177-1220-room-access-editing`.

## Scope

One deliverable: a Rust-owned access/history draft with a reusable
choice-and-detail interface in **Create Room** and **Room Info**, plus the
people-facing access label that reports the confirmed policy.

- #1177: "Members of selected Spaces" selectable and restorable with real
  membership allow conditions; `private` never presented as that route; the four
  history policies through the same interface; the combined outcome visible;
  selection separate from commitment.
- #1220: the room-list badge, the in-room header badge and the Room Info access
  summary say **Space members can join** only for the verified single-Space allow
  route and keep **Conditions apply** otherwise; Room Info stops summarising
  `restricted`/`knock_restricted` as `Private`; no invented names, no leaked IDs.

Non-goals: no permission-model expansion, no SDK fork, no round-trip machinery for
unmodelled condition types, no ordinary-room allow-target picker, no plural-parent
creation API, no second subscription or cache-repair owner, no redesign of the
Space access section shipped by #1166.

## Rust-owned draft (canon reconciliation, review item N1)

`REPOSITORY_RULES.md` requires state that decides a Matrix command's shape or its
selected target to be modelled as serializable Rust state first; a local React
scalar (`SettingsPropertyCard.tsx`'s single choice draft) is not a licence to
expand that into target selection. So the deliverable adds a serializable draft,
not a React-owned one:

- `RoomAccessDraft { scope, revision, rule, allow_targets, history, published_notes }`
  where `scope` is the room being edited or the pending create session, and
  `revision` increments on every accepted mutation.
- Typed commands set/clear the draft's rule, its allow target set and its history
  value; React keeps only DOM/focus state and raw text input drafts. All
  eligibility, conflict and comparison semantics live in Rust.
- The canonical comparison of "changed" is the draft against the confirmed policy
  as a **full value with a canonical target set** (sorted, deduplicated, so a
  reordered or duplicated server list is not a change).
- The preview is computed by Rust for the current draft revision, so an
  out-of-order preview result is fenced by the revision it was requested for.

This replaces the round-2 idea of React-owned selection and removes the need for
a canon exception.

## Verified current state

- `RoomSettingsSnapshot` carries `join_rule`, `history_visibility` and
  `permissions` only — no directory visibility, no allow-condition detail.
- `RoomJoinRule::is_settable` admits `Public | Invite | Knock | Private`; that
  restriction, and the reconciliation/cache rules beside it, are canon
  (`docs/architecture/state-machine.md:3758-3776`) and are amended here.
- `RoomSettingChange::JoinRule(RoomJoinRule)` cannot carry an allow list. The
  production write is an ordinary `update_join_rule(JoinRule)` state event.
- The restricted-condition projection loses information: it reports `Usable` as
  soon as one membership entry exists, keeps only membership IDs, and defaults an
  absent rule to `Invite`, so "membership only", "membership plus unsupported",
  "unsupported only" and "not inspected" are not distinguishable today.
- Target type and target name are different facts; a missing cached name falls
  back to the room ID and that fallback is currently carried into
  `access_allowed_room_names`. `is_space`/`room_type`/`create_content`
  availability are the authoritative signals, and a non-Space result is not
  positive proof of an unredacted ordinary room.
- Creation already sends a real membership allow condition for a standard private
  room in a Space (SDK create preset path, room version V9). What it cannot
  express is an independently selected policy and allow list: only `visibility`,
  `invitedOnly` and `parentSpace` are inputs, and the concrete presets, history
  override and publication are derived in the SDK adapter. The SDK currently
  **rejects** public plus invited-only.
- The in-room header is `components/panes.tsx`; both it and the room row
  special-case the existing generic label ID when building the tooltip. Room Info
  has its own binary Public/Private summary, no access tooltip, and suppresses
  access summaries for DMs. `knockRestricted` carries two routes (conditions and
  request) and the specific label must replace only the membership-route badge.
- `can_edit_settings` is a conjunction over name/topic/avatar/join rule/history;
  `can_change_join_rule` is event-specific and is what `allows_setting_change`
  uses for join rules. Room Info currently gates the join rule on the aggregate
  flag, blocking an account that may change join rules but not rename the room.
- Live coherence: `RoomAccessUpdated` updates only the access map, a successful
  setting change updates only `room_management.settings`, and the open-settings
  reconciliation searches Spaces only and compares only the join-rule enum, so an
  ordinary room's settings and an allow-list-only change are not reconciled.
- Directory publication is not projected anywhere; the SDK exposes
  `get_room_visibility`.

## Upstream comparison (corrected, with inspected revisions)

- **Element X Android** (`/tmp/exa`): `SecurityAndPrivacyView.kt` renders
  Space-member and combined request/member choices; `SecurityAndPrivacyPresenter.kt`
  manages authorised Spaces, maps to `JoinRule.Restricted`/`KnockRestricted` with
  membership conditions, folds joined/invited history, and adjusts directory
  visibility on save; `features/createroom/.../JoinRuleItem.kt` maps
  `PrivateVisibility.Restricted`/`AskToJoinRestricted` to the same rules with
  `AllowRule.RoomMembership(parentSpaceId)`.
- **Element X iOS** (`/tmp/exi`): `SecurityAndPrivacyScreenViewModel.swift`
  selects authorised Spaces and writes restricted / knock-restricted shapes and
  folds history; `View/SecurityAndPrivacyScreen.swift` renders the visible choice
  rows.
- **Element Web** (`/home/shinaoka/projects/Matrix/reference-repos/element-web`,
  inspected revision `48e4bce28e46b0161dbc8ca6b9dd2a3c2867d0d6`):
  `src/components/views/settings/JoinRuleSettings.tsx` renders a
  `StyledRadioGroup` for the access policy and implements a restricted-target
  management dialog with a real membership write;
  `src/components/views/settings/tabs/room/SecurityRoomSettingsTab.tsx` renders
  the history policy as a radio group too, writes history on selection, and can
  also change `worldReadable` history while changing public access to non-public.
  Revision 2's "each a select" claim was wrong and is withdrawn.
- **Intentional divergences**, recorded: per-property draft plus explicit Save
  versus the mobile screens' screen-level Save and Web's immediate commit on
  selection; four distinct history values exposed independently versus the mobile
  fold and Web's coupled history change; directory publication disclosed and never
  adjusted implicitly versus Android's save-time adjustment; stricter verified
  type and condition-completeness handling than the mobile mappings, which filter
  to membership entries.

## Rust-owned contract

1. **Condition completeness and availability.** One fact derived from the room's
   own join-rule content distinguishes: not inspected (the rule event is
   unavailable or the state is not synced), confirmed empty, membership only,
   membership plus unsupported, unsupported only. `Usable` may only be claimed for
   the membership-only case; an absent rule must never be defaulted into
   "inspected". The editor offers the membership editor only for membership only;
   any other case is displayed as such and an edit that would rewrite it is
   rejected with a typed failure and **zero state-event writes**.
2. **Re-validate immediately before mutation.** The completeness and availability
   check is repeated against current SDK content through the existing pre-send
   settings read in `crates/koushi-core/src/room/management.rs` (which must carry
   the new facts), not from the editor's loaded snapshot.
3. **Target classification and naming, kept separate.** `AllowTarget { kind,
   display_name }` with `kind` verified from the local room's create event
   (`Space`, `Room`, `Unknown` where the create event is missing, redacted or of
   unavailable type) and `display_name` optional and never a room ID. All distinct
   targets are counted **before** unnamed ones are dropped, and unnamed identities
   stay inside Rust when rejecting or preserving an edit.
4. **One pure outcome resolver, two contexts.** A pure function over (join policy,
   history policy, effective encryption, directory visibility, target facts,
   viewer facts) returning catalog IDs plus substitutions for "who can join",
   "who can read which history", "encrypted", "listed in the published
   directory". It must keep server history eligibility and key availability
   separate and must not imply that eligible users hold old decryption keys, and
   it keeps the existing non-retroactivity note.
   - Room Info: the property's own draft combined with the other property's
     **confirmed** value; the panel says which is which.
   - Creation: the complete **effective** proposed tuple after the same Rust
     normalization Create applies (so a retained private encryption draft or a
     public selection is reflected). The entry point resolves authoritative facts
     itself; React never supplies a trusted "verified Space" or "confirmed
     publication" assertion. Results are fenced by the draft revision.
   No new persistent outcome slice: confirmed outcomes extend the existing
   room-settings and access projections, and the preview is stateless.
5. **Settable restricted rules.** The draft carries the rule plus the selected
   allow Space IDs; the change is settable only with a verified allow list, and
   `private` stays reserved and is never offered as the Space route.
6. **Creation contract (closed, with precedence).**
   - Explicit policy controls join content; attachment stays independent, and an
     explicitly selected allow Space need not be the attachment Space. Choosing a
     target never requires permission on it and never implies writing
     `m.space.child`; the existing separate attachment/link failure report stays.
   - `visibility=public` with an explicit restricted policy is **rejected** with a
     reason (a published public room and a membership-restricted rule are
     contradictory, and the SDK already rejects public plus invited-only).
   - An explicit policy with `invitedOnly=true` is **rejected** with a reason; the
     explicit policy wins only when the legacy flags are at their defaults.
   - An explicitly **empty** allow list is **rejected**: the interface offers
     membership routes, not a restricted rule with no route.
   - History is optional: omitting it reproduces today's defaults; supplying it
     sends exactly one initial-state value.
   - The private option draft keeps its encryption/invitation choices while public
     is selected (existing behavior); only the effective submitted values are
     stripped. The preview uses the effective values.
   - Membership selection is allowed wherever the viewer is joined to at least one
     Space; the picker lists the viewer's joined Spaces. This is a product choice,
     not a Matrix restriction, and is stated as such.
   - When the explicit policy is absent the legacy private-in-Space preset path is
     preserved verbatim, including its room-version pin.
7. **Directory publication.** Read with `get_room_visibility`, scoped to the
   queried directory, projected as confirmed Public / confirmed Private /
   unavailable / loading / failed. A join-rule edit never changes it implicitly.
8. **Permissions.** `allows_setting_change` stays the single admission authority;
   the join-rule control renders from `can_change_join_rule`, and history keeps
   the aggregate permission with the divergence from Element documented.
9. **Live coherence and stale-cache transition.** Extend the existing access
   observation/reconciliation transition: reconcile ordinary rooms too, compare
   the full canonical policy, and preserve unrelated accepted local fields until
   their authoritative observation advances. Successful local access changes reach
   the shared projection without waiting for the SDK echo; external changes replace
   them when the observation advances. No second subscription, polling loop or
   cache-repair owner.
10. **Failure vocabulary.** Narrow policy-edit failure kinds distinguish
    "unsupported current condition" from "current policy could not be verified",
    through the protocol failure type and the per-property reducer status, replacing
    the generic SDK failure text for these two cases only.

## React contract

- One shared component (working name `AccessChoiceDetail`) used by Room Info's
  access/history section and by `CreateEntityDialog`, replacing the two
  `InlineChoicePropertyEditor` instances for these policies and the create
  dialog's visibility/invitedOnly controls. It reuses `SettingsPropertyCard`'s
  status/save affordances rather than a second status framework.
- Left: every relevant choice with a short label and a one-line summary, the
  selected one marked; right: the Rust outcome for the current draft, stating
  whether it describes confirmed or unsaved values. Unavailable choices state the
  concrete reason instead of being hidden.
- Save is **enabled** for a valid real change and disabled when unchanged, pending,
  forbidden or invalid. Room Info commits on Save, creation on Create, Cancel
  restores the confirmed value, pending/saved/failed/read-only stay on the
  property.
- Narrow and short windows stack list and details, keeping every choice and the
  actions reachable; keyboard navigation, visible focus, accessible
  selection/detail association, EN/JA and pseudo-locale hold.
- The three access surfaces consume one Rust-projected indicator contract
  (specific/generic fact plus tooltip substitutions): the room row, the in-room
  header in `panes.tsx`, and the Room Info summary. The row/header tooltip
  branches that special-case the current generic label ID move to the new
  contract; the `knockRestricted` request badge and its route survive; the
  summary keeps its navigation to the property and shows the confirmed value while
  the editor holds a draft; Room Info stops rendering restricted rules as
  `Private` and stops special-casing DMs.

## Acceptance criteria

Rust / state:

1. Not-inspected, confirmed-empty, membership-only, membership-plus-unsupported
   and unsupported-only are five distinct projections; a rejected edit writes
   nothing.
2. The completeness check is re-run before mutation: with the editor's draft open,
   change the server content to mixed-unsupported or make it unavailable, then
   submit → typed failure, zero join-rule writes.
3. A verified single-Space route yields the specific sentence and a safe Space
   name; an ordinary-room target, several targets, an unknown or redacted type, an
   unavailable name, or no usable condition yields the generic sentence with no raw
   ID; `private` is never the Space route.
4. Setting restricted with a verified allow list round-trips; without one it is
   rejected; reordered or duplicated server targets are not a change, a genuinely
   changed target is.
5. Creation: membership choice in a Space produces a server room whose join rule
   carries that Space; explicit history is sent once; omitting both reproduces
   today's presets; public plus explicit restricted, explicit policy plus
   `invitedOnly`, and an explicitly empty allow list are each rejected with a
   reason; a selected allow Space different from the attachment is honoured with
   no permission on that Space.
6. `can_change_join_rule = true, can_edit_settings = false` submits a join-rule
   change successfully; a forbidden direct command is still rejected.
7. Another client changing only the allow targets, and another changing history,
   converge in open settings without a reload or Space change; a history save
   immediately after an access save (and vice versa) never carries the other
   property's stale value.
8. Directory publication is reported as confirmed Public / confirmed Private /
   unavailable / loading / failed and is unchanged by a join-rule edit.
9. The resolver separates history eligibility from key availability and keeps the
   non-retroactivity note for all four policies.

Frontend:

10. Both surfaces render the same structure: every choice reachable, selection
    distinct from the confirmed value, details following the current draft,
    correct Save enablement, per-surface commit semantics, per-property status,
    and no stale details after an out-of-order preview result.
11. The three access surfaces agree for one room in Home, a Space, People/DM,
    favourites and low-priority contexts; the tooltip names the Space on hover and
    keyboard focus; both `knockRestricted` badges survive.
12. Cancellation, failed save and retry, and read-only states behave per the
    property in both surfaces.

Production path:

13. The browser payload submitted from each surface is asserted, not only the
    standalone component.
14. `qa:headless-local` on both servers, extended in the existing `room_management`
    scenario (its `space_access` check currently observes only scalar join rules):
    `npm --prefix apps/desktop run qa:headless-local -- --server=both --core --scenario=room_management --timeout-ms=240000`
    asserts create with the membership choice and reads back the synced allow
    content and history; restores a restricted rule through the real update
    command and reads back the synced allow content; observes a second-client
    allow-target and history change without refresh.
15. Two parent attachments are produced by creating with one and adding the second
    through the existing link command, then asserting the allow content is exactly
    the selected target.
16. Narrow **and short** viewports with Japanese, English, accented and bidi
    pseudo-locales keep every choice and the actions reachable.

Each behaviour records the RED check first (command, exit status, captured
failure) and then the same check GREEN.

## Contract surfaces to mirror

Follow the exhaustive inventory in `docs/agents/state-ownership.md` and then these
production integration points: `crates/koushi-state/src/state/room_management.rs`
(snapshot, draft, change enum, canonical comparison, outcome),
`crates/koushi-state/src/sidebar.rs` and the room-access projection,
`crates/koushi-core/src/room/{operations.rs,management.rs,list_observer.rs}` and
the reconciliation path, `crates/koushi-core/src/runtime/connection.rs` for the
stateless preview route, `crates/koushi-sdk/src/{room_operations.rs,
room_projection.rs}` (write adapter and success projection — today's conversion
rejects restricted rules), `crates/koushi-protocol/src/command/room.rs` and
`crates/koushi-protocol/src/event/room.rs`,
`apps/desktop/src-tauri/src/dto.rs` and `commands/contracts.rs` (command contract
tests) plus the preview command registration, `apps/desktop/src/backend/{desktopApi.ts,client.ts}`,
`apps/desktop/src/App.tsx` (real request mapping and draft commands),
`apps/desktop/src/domain/{types.ts,accessCondition.ts,coreEvents.generated.json}`,
`apps/desktop/src/components/{RoomInfoPanel.tsx,dialogs.tsx,panes.tsx,Shell.tsx,SettingsPropertyCard.tsx}`,
fixtures with real non-empty restricted content, the `frontend_app_state` golden,
the forwarded state-delta fixture, the canon (`docs/architecture/state-machine.md`,
including the settable-rule list and the reconciliation/success-projection
sections), and the user guide.

## Verification plan (iteration)

Focused `cargo test -p koushi-state -p koushi-core -p koushi-protocol -p koushi-sdk`,
`cargo fmt --check`, clippy `-D warnings` for changed crates, golden regenerate +
normal run, `npm run typecheck`, `npm run lint`, focused `npx vitest run`, the
touched Playwright specs. Landing gate: the feature-unified workspace suite plus
the extended `room_management` QA command above, run on both servers.

## Delete list (enforced)

- No independent persistent outcome slice; no React-owned policy/target state.
- No new Matrix write mechanism or SDK fork.
- No unsupported-condition round-trip; reject with the completeness fact.
- No ordinary-room allow-target picker; preserve or reject existing ones.
- No plural-parent creation API just for QA; use the existing child-link command.
- No second property-card/status/focus framework; reuse `SettingsPropertyCard`.
- No duplicate Tauri settings DTOs; register only the genuinely new preview command.
- No second policy subscription, polling loop or cache-repair owner.
- No permission-model expansion; keep the aggregate history gate and document it.
- No speculative creation modes; defer `knock_restricted` creation unless its
  supported room-version answer is established, and keep existing knock-restricted
  display and request behavior meanwhile.

## Open questions for the reviewer

1. Whether deferring `knock_restricted` creation is acceptable, given the private
   in-Space preset pins room version V9 and a combined request/member route needs
   its supported-version answer.
2. Whether the "membership selection anywhere the viewer has a joined Space" rule
   is the right product behavior, or whether creation should stay Space-attached
   only.
