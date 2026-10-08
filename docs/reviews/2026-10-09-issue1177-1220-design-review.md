# Independent design review — #1177 + #1220 (revision 1)

Reviewer: GPT-6.1 Sol, read-only, 2026-10-08T21:39:29Z. Verdict: revise before implementation (3 blockers).
This file is the review record for `docs/plans/2026-10-09-issue1177-1220-room-access-editing.md` revision 1; revision 2 must fold it in.

## Verdict: revise before implementation

Revision 1 captures the intended UI, but several contracts are not sufficiently defined to implement safely. In particular, the existing allow-condition projection loses information needed for safe editing, the create-request extension does not fully describe the new access selection, and no production path is specified for Rust-generated **draft** previews.

I read the design completely first, then inspected the relevant state, Core, SDK adapter, protocol, Tauri, frontend, and existing test paths. This was read-only; no builds or tests were run.

Below, **“plan”** means `docs/plans/2026-10-09-issue1177-1220-room-access-editing.md`.

## Findings

### 1. BLOCKER — Existing restricted-condition facts cannot support the promised preservation/rejection behavior

**Plan:** lines 84–96, 110–112, 143–150.

The current classification does **not** retain whether unsupported entries coexist with usable membership conditions:

- `crates/koushi-sdk/src/room_projection.rs:1974–1991`: `matrix_room_restricted_conditions` returns `Usable` immediately when any membership entry exists, before inspecting unsupported entries.
- `crates/koushi-sdk/src/room_projection.rs:2001–2023`: `matrix_room_restricted_allow_room_ids` retains only membership entries.
- `crates/koushi-state/src/state/room_management.rs:276–284`: `RoomAccessCondition` has no independent unsupported-entry fact or original condition representation.

Consequently, these two server policies project identically:

1. one Space membership condition;
2. that same membership condition plus an unsupported condition.

An editor built from this projection can silently drop unsupported entries. A badge can also make the specific single-Space claim despite the plan’s promise that unsupported allow data remains generic.

**Smallest correct alternative:** retain an independent completeness/unsupported-condition fact from the actual join-rule content. Reject restricted-policy editing when unsupported entries exist; round-tripping arbitrary condition types is unnecessary for this deliverable. Perform that check again against current SDK state before sending.

**Required acceptance:** a mixed membership-plus-unsupported list, not merely an unsupported-only list; submission must produce no state-event write when rejected.

---

### 2. BLOCKER — The creation extension lacks an explicit access contract and precedence rules

**Plan:** lines 53–59, 97–103, 119–122.

Adding history visibility and an optional membership list leaves creation governed by three existing inputs: `visibility`, `invitedOnly`, and `parentSpace`. The design does not say how the new choice overrides—or conflicts with—those inputs.

The production path is:

- `apps/desktop/src/App.tsx:426–443`: constructs the request; public selection strips encryption and `invitedOnly`, and attachment comes from the active Space.
- `crates/koushi-protocol/src/command/room.rs:12–26`: exposes those existing fields.
- `crates/koushi-core/src/room/operations.rs:155–174`: copies them into SDK options.
- **`crates/koushi-sdk/src/room_operations.rs:834–939`**, not Core’s conversion, derives the actual presets, join conditions, history override, encryption, and directory visibility.

Creation **already sends a real membership allow condition** for a standard private room in a Space (`:917–928`). The missing capability is an **explicit, independently selected policy/list**, not membership-based creation itself.

Undefined combinations include:

- public visibility plus a membership list;
- `invitedOnly=true` plus a membership list;
- membership selection at Home;
- a selected allow Space different from the attachment Space;
- omitted list versus explicitly empty list;
- switching choices while retaining the old private encryption draft.

The legacy private-in-Space path also derives its allow target from attachment. “Current callers keep working” and “attachment never silently becomes the allow list” therefore need an explicit compatibility boundary.

**Smallest correct alternative:** add an optional explicit access-policy override, with a closed supported shape and documented legacy fallback when omitted. The new UI always submits its explicit policy. Treat attachment separately. Resolve history defaults once, then apply one explicit override without duplicate initial-state events.

Do not impose permission to edit the target Space: choosing it as an allow target and writing its `m.space.child` event are different operations. Existing creation deliberately reports linking failure separately without failing creation (`crates/koushi-core/src/room/operations.rs:403–449`).

---

### 3. BLOCKER — There is no specified production route for the Rust-owned unsaved outcome

**Plan:** lines 81–82, 104–109, 123–131; open question at 201–203.

An outcome attached to a confirmed `RoomSettingsSnapshot` cannot explain an unsaved choice, much less a room that does not yet exist. The design promises that React does not derive outcomes, but supplies no preview request, input shape, or result-delivery contract.

Today:

- `SettingsPropertyCard.tsx:320–390` holds a local choice draft and renders notes for that draft.
- `RoomInfoPanel.tsx:442–482` generates history notes locally.
- `RoomSettingsSnapshot` contains confirmed settings only (`crates/koushi-state/src/state/room_management.rs:90–106`).

**Smallest correct alternative:** specify one pure Rust outcome resolver over the proposed access/history policy and required contextual facts, and one existing-style stateless Core/adapter preview entry point. The room-address preview is an established precedent (`apps/desktop/src-tauri/src/commands/room.rs:779–789`; `crates/koushi-state/src/room_address.rs:5–18`).

Specify which policy pair a property preview describes: its draft plus the other **confirmed** policy, or both drafts. With independently saved properties, this distinction must be visible; otherwise Save may commit a different combined result from the one displayed.

Do not introduce a new persistent outcome slice merely to solve draft rendering.

---

### 4. IMPORTANT — Directory publication is not currently available as a confirmed boolean

**Plan:** lines 28–31, 104–106.

Neither `RoomSettingsSnapshot` nor the current SDK settings projection contains directory visibility:

- `crates/koushi-state/src/state/room_management.rs:90–106`;
- `crates/koushi-sdk/src/room_projection.rs:1653–1685`.

Public join rules, aliases, encryption, and Space attachment are not proof that the room is published. Creation does set request visibility (`crates/koushi-sdk/src/room_operations.rs:869–872`), but existing rooms require a separate read.

The public SDK already provides the exact mechanism:

- `vendor/matrix-rust-sdk/crates/matrix-sdk/src/room/privacy_settings.rs:162–169`: `get_room_visibility`.

**Smallest correct alternative:** add the SDK read to the relevant settings load and represent unavailable/loading/failed data distinctly from confirmed unpublished. Scope the displayed fact to the queried directory. Revise the non-goal that currently restricts Matrix calls to join-rule/“delayed-state” calls.

**Required acceptance:** public-but-unpublished, invite-only-but-published, and failed directory read; changing join rules must not implicitly change directory publication unless explicitly agreed.

---

### 5. IMPORTANT — “Existing permission separation” is overstated, and Room Info currently uses the wrong join-rule gate

**Plan:** lines 28, 75–77, 136–137.

The permission facts are not independently event-specific for both policies:

- `crates/koushi-sdk/src/room_projection.rs:1630–1645`: `can_edit_settings` is a conjunction of permission for name, topic, avatar, join rules, and history visibility.
- `:1647–1650`: `can_change_join_rule` is independently event-specific.
- `crates/koushi-state/src/state/room_management.rs:308–315`: `allows_setting_change` uses the latter for join rules, but the former for history.

Room Info nevertheless uses `mayEditSettings` and the common `submitSetting` guard for join rules (`RoomInfoPanel.tsx:172–188`, `:434–440`). Thus a caller who may change join rules but may not rename the room is blocked by the renderer, although Core authorizes the command.

`SpaceAccessSection.tsx:91` already uses the correct join-rule permission.

**Smallest correct alternative:** reuse `RoomPermissionFacts::allows_setting_change` as authoritative admission and render join-rule controls from `can_change_join_rule`. Keep history under the existing aggregate permission if “no permission-model change” is genuinely the agreed scope, but document that divergence from Element. Do not claim independent history-event permission without adding and reviewing that contract.

**Required acceptance:** `can_change_join_rule=true`, `can_edit_settings=false`, including a real submitted command; also test direct forbidden commands.

---

### 6. IMPORTANT — The shared live projection is a requirement, not an implementation plan

**Plan:** lines 113–115, 156–157.

Current Room Info and sidebar access do not share one owner:

- `crates/koushi-state/src/reducer/room_management.rs:70–94`: successful setting changes update only `room_management.settings`.
- `crates/koushi-state/src/reducer/room.rs:34–82`: `RoomAccessUpdated` updates only the access map.
- `crates/koushi-state/src/reducer/room.rs:147–170`: open-settings reconciliation searches **Spaces only** and compares only the join-rule enum.
- `crates/koushi-core/src/room/list_observer.rs:1534–1567`: access facts are produced from live SDK rooms.
- `crates/koushi-core/src/state_delta.rs:428–430`: access-map changes already trigger sidebar publication.

The Space reconciliation does not handle an ordinary room’s settings, or changes from `restricted(A)` to `restricted(B)` with the enum unchanged.

**Smallest correct alternative:** extend the existing access observation/reconciliation path to complete policy facts, not a second subscription owner. Specify how accepted local writes coexist with an older SDK cache until echo, and how another-client updates refresh open settings.

**Required acceptance:** another client changes only the allow targets; Room Info remains open; no reload or active-Space change is used to obtain convergence. Include an external history-policy change because it affects the combined preview too.

---

### 7. IMPORTANT — Existing “unchanged” and “saved” checks compare only the rule, not the allow policy

**Plan:** lines 92–96, 128–131, 161–164.

Current attribution records a scalar target:

- `RoomInfoPanel.tsx:120–128`;
- `:195–227`: confirmed join value and saved comparison use only `settings.join_rule`.
- `SettingsPropertyCard.tsx:348–353`, `:374–379`: equality disables Save.

A change from one selected Space to another keeps `restricted` unchanged. Reusing these mechanisms unchanged can disable Save or report Saved before the new list is confirmed.

The promised clear unsupported-condition failure is also not currently wired:

- `crates/koushi-core/src/room/operations.rs:225–228`: invalid room settings become `RoomFailureKind::Sdk`.
- `RoomInfoPanel.tsx:728–730`: every non-forbidden failure gets generic operation-failed text.

**Smallest correct alternative:** compare a canonical full access-policy value and retain existing request correlation. Add only the narrow failure discriminator needed to explain unsupported/unavailable policy edits; do not build a new general property-operation framework.

---

### 8. IMPORTANT — The badge/header/summary integration omits actual consumer details

**Plan:** lines 104–109, 175–183.

The in-room header is in **`apps/desktop/src/components/panes.tsx`**, not `Shell.tsx`:

- `panes.tsx:838–845`: resolves access through `sidebarRoomAccess`;
- `:1029–1046`: renders header badges and tooltips.
- `domain/accessCondition.ts:184–210`: `sidebarRoomAccess` searches the legacy convenience vectors.

Both header and row tooltip rendering special-case the literal existing label ID:

- `Shell.tsx:1409–1414`;
- `panes.tsx:1034–1040`.

Changing the label to “Space members can join” without changing these branches loses the tooltip’s Space name.

Room Info has a separate binary Public/Private implementation and no access tooltip (`RoomInfoPanel.tsx:294–322`, `:737–757`). It also suppresses access summaries for DMs at `:751`, whereas the row/header access vocabulary deliberately does not depend on DM classification.

Finally, `knockRestricted` currently carries **two routes**, conditions and request (`domain/accessCondition.ts:101–125`). The new specific label must replace only the membership-route badge, not erase “Can request.”

**Smallest correct alternative:** extend the existing shared indicator contract with Rust-projected specific/generic facts and tooltip substitutions; consume it in all three surfaces. Preserve summary-to-property navigation and the request route.

**Required acceptance:** named tooltip on hover and keyboard focus; both knock-restricted badges; Home, Space, People/DM, favourites and low-priority contexts; summary remains confirmed while the editor contains an unsaved draft.

---

### 9. IMPORTANT — Target type and display-name availability must remain separate; existing display labels can be raw IDs

**Plan:** lines 84–91.

“Name unavailable → use `Unknown`” conflates an unknown room type with a verified Space whose name is unavailable. Those facts have different meanings for safe editing and diagnostics.

The SDK supports authoritative inspection without a fork:

- `vendor/matrix-rust-sdk/crates/matrix-sdk-base/src/room/mod.rs:222–236`: `is_space` / `room_type`;
- `:325–335`: optional `create_content`, including the documented missing/redacted-event caveat.

`room_type()==None` alone does not distinguish an ordinary room from missing create data. Likewise, `is_space()==false` is not positive ordinary-room verification.

The current naming pipeline is also weaker than the plan suggests:

- `crates/koushi-sdk/src/room_projection.rs:2318–2324`: missing cached display name falls back to the room ID.
- `crates/koushi-core/src/room/normalization.rs:23–26`: carries that Space display name through.
- `crates/koushi-state/src/sidebar.rs:260–267`, `:648–663`: treats it as an allowed-target name.

Thus “resolved display label” is not sufficient proof of a safe accessible Space name.

**Smallest correct alternative:** retain verified type separately from optional safe display name; use explicit create-event availability when classifying. Count all distinct targets **before** filtering unnamed targets. Keep unnamed identities inside Rust when rejecting/preserving edits; do not invent a name or expose an ID to explain them.

---

### 10. IMPORTANT — The mirror inventory misses the SDK adapter and event artifact, while suggesting unnecessary adapter work

**Plan:** lines 175–189.

The app-owned enum change cannot reach the server without changing:

- `crates/koushi-sdk/src/room_operations.rs:260–266`, `:329–334`: SDK adapter change type and execution;
- `crates/koushi-sdk/src/room_projection.rs:1919–1920`, `:2037–2051`: success projection and conversion that currently rejects restricted rules;
- `crates/koushi-core/src/room/management.rs:155–167`: conversion between the two enums.

The finished snapshot also appears in events, not just the full-state golden:

- `crates/koushi-protocol/src/event/room.rs:124–130`;
- `apps/desktop/src-tauri/src/core_event_forwarder/tests.rs:1178–1185`.

The mirror list should explicitly include `coreEvents.generated.json`, relevant event fixtures, command contract tests, and the real `App.tsx` request mapping. `docs/agents/state-ownership.md:55–102` already defines this inventory and regeneration discipline.

Conversely, nested settings already travel directly as `RoomManagementState`:

- `dto.rs:316`, `:515`, `:617`, `:694`.

A nested settings field does **not** require a duplicate Tauri settings DTO. Existing `create_room` and `update_room_setting` commands are already registered paths; command registration changes are needed only if a new preview command is introduced.

**Smallest correct alternative:** follow the existing mirror checklist, populate nonempty restricted-policy fixtures, and extend existing transport paths rather than creating parallel DTOs.

---

### 11. IMPORTANT — Acceptance needs explicit production-path evidence, not only projected-state tests

**Plan:** lines 143–170, 187–192.

The prose mentions server-side results, but the verification plan does not identify a focused Core QA scenario or production-path oracle. Several existing tests demonstrate why this matters:

- `apps/desktop/e2e/room-access-conditions.spec.ts:84–133` directly pushes sidebar snapshots. These can prove rendering while another-client observation remains broken.
- `crates/koushi-sdk/src/room_operations.rs:344` returns a submitted-change projection after sending. A “round-trip” assertion against that result does not prove synced/server content.
- `crates/koushi-qa/src/bin/headless_core_qa/scenarios/rooms/space_access.rs:102–113` already uses the stronger pattern: observe the synced rule, then reload.

**Add concrete acceptance using existing infrastructure:**

- Create through `CoreCommand::Room(CreateRoom)` on both local servers; read back actual join-rule **allow content** and history visibility.
- Exercise restore through the real update command, then inspect synced/server allow content.
- Observe a second-client rule/allow-list change without manual refresh.
- Assert browser submission payloads from both surfaces—not only the standalone shared component.
- Test two parent attachments but exactly one explicitly selected allow Space.
- Test confirmed empty, unsupported-only, mixed unsupported, and unavailable data separately.
- Assert all four history choices, encrypted key limitations, non-retroactivity, and directory uncertainty.
- Measure/click reachable controls at narrow **and short** viewport sizes, including accented and bidi pseudo-locales; `toBeVisible()` alone is insufficient (`docs/policies/engineering-rules.md:733–748`).

A full local QA gate does not substitute for a scenario that asserts these new contracts.

## Upstream comparison

### 12. IMPORTANT — The Android claim is false in the available source; iOS and meaningful divergences are missing

**Plan:** lines 64–77.

The available Element X Android checkout contains restricted-policy editing:

- `/tmp/exa/features/securityandprivacy/impl/src/main/kotlin/io/element/android/features/securityandprivacy/impl/root/SecurityAndPrivacyView.kt:234–265`: Space-member and combined request/member choices.
- The corresponding `SecurityAndPrivacyPresenter.kt:275–341`: selected Space handling and authorized-Space management.
- `:475–484`: maps selections to `JoinRule.Restricted` / `KnockRestricted` with membership conditions.

Android creation also supports both variants:

- `/tmp/exa/features/createroom/impl/src/main/kotlin/io/element/android/features/createroom/impl/configureroom/ConfigureRoomPresenter.kt:155–158`;
- `JoinRuleItem.kt:39–42`.

Element X iOS has the same relevant flow:

- `/tmp/exi/ElementX/Sources/Screens/SecurityAndPrivacyScreen/SecurityAndPrivacyScreenViewModel.swift:291–325`: authorized-Space selection and existing non-parent targets;
- `:339–352`: restricted and knock-restricted write shapes.
- `View/SecurityAndPrivacyScreen.swift:44–87`: visible choices with descriptions.

Missing intentional divergences include:

- independent per-property Save versus mobile screen-level Save;
- four distinct history values versus mobile folding joined/invited (`Android SecurityAndPrivacyPresenter.kt:488–506`; iOS view model `:355–377`);
- preserving directory publication versus Android’s save-time visibility adjustment (`Android presenter :414–431`);
- stronger verified-type/unsupported-condition handling than the mobile mappings, which filter membership entries.

`REPOSITORY_RULES.md:118–125` requires Element Web **and Android/iOS**, observed command/state shape and UX, and documented divergences. This section does not meet that requirement.

I could not independently verify the Element Web claims from available source. The cited test path is not enough evidence for implementation details or negative claims such as “no allow-list editor beyond the parent-Space option.” Record actual implementation references and inspected revisions.

### 13. MINOR — Correct the canon/update inventory and the “delayed-state” wording

The plan changes what can be sent, while `docs/architecture/state-machine.md:3758–3763` explicitly restricts settable rules to the current four. Add that canon amendment to the deliverable, along with revised snapshot/reconciliation semantics.

The existing production mutation is ordinary `privacy_settings().update_join_rule` (`crates/koushi-sdk/src/room_operations.rs:329–334`), implemented by an ordinary state-event request (`vendor/matrix-rust-sdk/crates/matrix-sdk/src/room/privacy_settings.rs:131–138`). “Delayed-state calls” at plan line 29 is inaccurate and unnecessary.

## Answers to the open questions

- **Spaces-only selection:** yes, for newly selected targets. Preserve existing ordinary-room targets faithfully or reject the edit clearly; never silently discard them.
- **`knock_restricted` creation:** not required to invent a broader creation feature for these issues. If deferred, show the reason and preserve existing rules in Room Info. If included, explicitly resolve supported room versions—the current private-in-Space path fixes version V9 (`crates/koushi-sdk/src/room_operations.rs:915–916`).
- **Outcome location:** confirmed detailed outcomes can live with settings; compact badge facts belong in the existing access/sidebar projection. A pure resolver can be shared with draft previews. A new independent state slice is not justified.

## Delete list

Read both requested ponytail skill files before this pass.

14. **IMPORTANT — plan:201–203 — `reuse:` Delete the proposed independent outcome slice; extend existing access/sidebar facts and use a pure outcome resolver.**
15. **MINOR — plan:92–96 — `reuse:` Delete any new Matrix write mechanism or SDK fork; `RoomPrivacySettings::update_join_rule(JoinRule)` already accepts full restricted content.**
16. **IMPORTANT — plan:110–112 — `shrink:` Delete arbitrary unsupported-condition round-trip machinery; reject affected edits explicitly using current-source completeness facts.**
17. **MINOR — plan:196–198 — `yagni:` Delete ordinary-room target selection from this deliverable; preserve/reject existing ordinary-room conditions instead.**
18. **MINOR — plan:199–200 — `yagni:` Delete speculative creation modes and capability frameworks; expose only agreed, implemented routes with concrete unavailable reasons.**
19. **MINOR — plan:119–131 — `reuse:` Delete a second property-card/status/focus framework; reuse `SettingsPropertyCard` and existing edit-focus behavior around native choice controls.**
20. **MINOR — plan:175–183 — `reuse:` Delete duplicate Tauri settings DTOs and mechanical command-registration changes; existing nested state and registered commands already cover those paths.**

**Net:** implementation lines saved cannot be quantified before a diff exists; these are avoided implementation surfaces, not a measured line reduction.

## What I did not check

- No builds, tests, QA scenarios, browser sessions, or homeserver requests were run.
- No files were edited.
- I did not verify branch ancestry, the stated baseline SHA, or submodule gitlink identity with Git commands.
- I did not independently inspect Element Web implementation/source or verify the revisions of the available Android/iOS checkouts.
- I did not verify native macOS/Windows layout, assistive-technology behavior, or real-server support for every proposed policy combination.
- I inspected relevant production paths and selected existing tests, not every file or every test in the repository.
