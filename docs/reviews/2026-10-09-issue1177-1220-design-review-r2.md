# Independent design review — #1177 + #1220 (revision 2)

Reviewer: GPT-6.1 Sol, read-only. Round 2; round 3 of the design folds it in.

# Round-2 pre-implementation review

**Verdict: revise before implementation.** Revision 2 substantially improves the design, but the creation contract and creation preview still admit incompatible implementations. The condition-projection contract also remains incomplete for unavailable data and submission-time validation.

Reviewed:
- `docs/reviews/2026-10-09-issue1177-1220-design-review.md`
- `docs/plans/2026-10-09-issue1177-1220-room-access-editing.md`, completely
- Relevant canon, production callers/adapters/reducers, and selected tests

The worktree’s Git metadata identifies branch `feat/1177-1220-room-access-editing` at **`3f587c41be5a8f1ff1c7de3b554aa127e51fef30`**, with the revision-2 commit recorded in its HEAD reflog. I did not execute `git log`.

Below, **plan** means `docs/plans/2026-10-09-issue1177-1220-room-access-editing.md`. “Resolved” means resolved **as a design commitment**, not implemented or verified.

## Resolution of round-1 items

### 1. BLOCKER — **Partially resolved:** mixed unsupported conditions are retained, but unavailable data and fresh validation remain unspecified

**Revision 2:** plan:118–124 now requires an independent fact distinguishing:

> “confirmed-empty, membership-only, membership-plus-unsupported, and unsupported-only”

and rejects affected edits with:

> “no state-event write”

This genuinely fixes the central mixed-condition modelling omission.

**Source:** `crates/koushi-sdk/src/room_projection.rs:1974–1991` currently returns `Usable` as soon as a membership entry exists; `:2001–2023` extracts only membership IDs. `crates/koushi-state/src/state/room_management.rs:276–284` carries no independent unsupported-content fact. The revision correctly identifies those defects.

**Still open:**

- The new four-way contract has **no unavailable/uninspected state**. Unknown **target type** at plan:125–131 is not unavailable **join-rule/allow content**.
- The plan does not explicitly require the completeness check against current SDK state immediately before mutation. Rejecting from the editor’s loaded snapshot alone is insufficient after another client changes the policy.

The distinction exists in the SDK: `vendor/matrix-rust-sdk/crates/matrix-sdk-base/src/room/room_info.rs:1133–1137` returns a join rule only when the event is available. `vendor/matrix-rust-sdk/crates/matrix-sdk-base/src/room/mod.rs:281–297` separately exposes state-sync availability. The current app helper defaults an absent rule to Invite (`crates/koushi-sdk/src/room_projection.rs:1937–1942`), which must not become proof of inspected, supported content during editing.

**Smallest correction:** add one availability/completeness fact; reject policy rewrites when current source content cannot be safely inspected. Reuse the existing pre-send settings read in `crates/koushi-core/src/room/management.rs:232–260`, with the necessary SDK facts included. No generic validation framework or unsupported-content serializer is needed.

**Acceptance:** open a supported policy, change its server content to mixed unsupported or make its authoritative content unavailable, then submit the old draft; assert the typed failure and **zero join-rule writes**.

---

### 2. BLOCKER — **Partially resolved:** creation names the missing cases but still does not define their behavior

**Revision 2:** plan:150–162 adds an optional explicit policy, preserves the omitted-policy fallback, permits an allow Space different from attachment, and correctly says:

> “Choosing a Space as an allow target never requires permission on that Space”

Those are real improvements.

**Source:** the compatibility boundary is necessary because:

- `apps/desktop/src/App.tsx:426–443` currently derives alias, encryption, invitation mode and attachment from visibility and active Space.
- `crates/koushi-protocol/src/command/room.rs:12–26` exposes those legacy inputs.
- `crates/koushi-sdk/src/room_operations.rs:847–875` derives presets/publication from visibility and invitation mode.
- `:914–939` independently derives V9, restricted membership and Invited history from attachment.
- `crates/koushi-core/src/room/operations.rs:403–449` correctly separates failed attachment from successful creation.

**Still undefined:**

1. Does an explicit access policy override `invitedOnly`, or does a conflicting legacy flag reject the request?
2. What happens for `visibility=public` with an explicit restricted policy? Does visibility remain directory publication, continue to select the old preset, or make this combination invalid?
3. What does **explicitly empty** mean? The plan says it differs from omission, but does not state whether it is rejected or produces restricted-with-no-membership-route.
4. Is the history field optional? The text says “an explicit history policy,” while acceptance at plan:226–227 requires omitting it to reproduce defaults.
5. How is the existing encryption draft retained when changing choices?

The wording:

> “`visibility`/`invitedOnly` keep their current public-room stripping”

does not settle precedence. Moreover, stripping happens in **App’s mapping**; the SDK currently **rejects** public plus invited-only (`room_operations.rs:847–850`).

Encryption retention is observable existing behavior: `apps/desktop/src/components/dialogs.tsx:311–318` deliberately preserves private options while public is selected, whereas `App.tsx:436–437` strips only the effective submitted values.

**New scope restriction:** plan:155 prohibits membership selection at Home. An active attachment Space is not inherently required to construct a membership allow condition; the SDK’s current coupling is the legacy implementation, not a demonstrated Matrix restriction. If this is an intentional product limitation, state its actual reason and obtain agreement—do not present it as inevitable semantics. “Authorised Space” at plan:156 also needs a concrete definition consistent with the explicit no-target-permission rule.

**Smallest correction:** specify one closed explicit-policy contract and a short precedence table. Explicit policy controls join content; attachment remains independent; conflicting legacy combinations are either explicitly ignored or explicitly rejected. State the empty-list result, optional history behavior and draft-retention rule. Keep the legacy path only when the override is absent.

---

### 3. BLOCKER — **Partially resolved:** Room Info preview is defined; Create Room preview is not

**Revision 2:** plan:132–142 now specifies a pure resolver and a stateless entry point, without an independent outcome slice. For Room Info it correctly chooses:

> “the property’s own draft combined with the other property’s confirmed value”

**Source:** the precedent is real:

- `apps/desktop/src-tauri/src/commands/room.rs:779–789` forwards room-address preview through Core.
- `crates/koushi-core/src/runtime/connection.rs:1125–1151` resolves against current account/scope facts.
- `crates/koushi-state/src/room_address.rs:5–18` supplies a typed result.

**Remaining contradiction:** plan:191–194 applies “draft-combined-with-confirmed” to the shared component, but **creation has no confirmed policy**. Create commits access, history and encryption together. A preview that combines access draft with a default rather than the selected history draft can describe a different room from the one Create submits.

The creation resolver must also use **effective** encryption/publication after request normalization, not retained private draft values. That follows directly from `App.tsx:436–438`.

**Smallest correction:** define two contexts for the same resolver:

- **Room Info:** property draft + other confirmed policy.
- **Creation:** complete effective proposed access/history/encryption/publication tuple after the same Rust normalization used by Create.

Specify that the Core preview entry point obtains authoritative contextual facts; React must not supply “verified Space” or “confirmed publication” assertions as trusted input. Add ordinary draft-identity fencing so a late result cannot replace details for a newer selection.

**Acceptance:** change both creation policies, toggle public/private with a retained encryption draft, and assert that preview and submitted effective policy agree. Cover out-of-order preview responses.

---

### 4. IMPORTANT — **Resolved:** directory publication is now an explicit read with uncertainty

**Revision 2:** plan:163–166 requires:

> “SDK’s `get_room_visibility` … scoped to the queried directory”

with confirmed/unavailable/loading/failed distinct and no implicit publication mutation.

**Source:** `vendor/matrix-rust-sdk/crates/matrix-sdk/src/room/privacy_settings.rs:162–169` performs exactly that directory read. `crates/koushi-state/src/state/room_management.rs:90–106` and `crates/koushi-sdk/src/room_projection.rs:1653–1685` currently lack the fact.

The old non-goal restricting calls to the mutation mechanism is gone. This is resolved at design level.

**Acceptance still needs concrete cases:** public-but-unpublished, invite-only-but-published, and failed read. “Confirmed” must mean a confirmed **Public or Private value**, not merely a successful request.

---

### 5. IMPORTANT — **Resolved:** join-rule permission and history divergence are correctly identified

**Revision 2:** plan:167–171 keeps:

> “`RoomPermissionFacts::allows_setting_change` … the single admission authority”

and renders join editing from `can_change_join_rule`, while explicitly keeping aggregate history permission.

**Source:**

- `crates/koushi-sdk/src/room_projection.rs:1630–1650` contains the conjunction versus event-specific facts.
- `crates/koushi-state/src/state/room_management.rs:308–315` is the authoritative helper.
- `apps/desktop/src/components/RoomInfoPanel.tsx:172–188,434–440` currently applies the aggregate gate.
- `crates/koushi-core/src/room/management.rs:244–260` uses the helper before SDK mutation.

Acceptance at plan:229–230 includes the asymmetric permission case and direct forbidden command.

**Minor clarification:** plan:309–311 reopens history permission despite plan:169–171 already selecting the minimal scope. Keep the aggregate permission for this deliverable unless the user explicitly expands scope.

---

### 6. IMPORTANT — **Partially resolved:** the correct observation owner is named, but stale-cache handling is still an aspiration

**Revision 2:** plan:172–178 says:

> “Extend the existing access observation and reconciliation path”

including ordinary rooms, full policy comparison and external history changes. This resolves the second-owner concern.

**Source:**

- `crates/koushi-core/src/room/list_observer.rs:1534–1567` produces the existing access projection.
- `crates/koushi-state/src/reducer/room.rs:34–82` installs that projection.
- `:147–170` currently reconciles only Space join-rule transitions.
- `crates/koushi-state/src/reducer/room_management.rs:70–94` installs a separate success snapshot.
- `crates/koushi-core/src/state_delta.rs:428–430` already publishes sidebar changes for access-map changes.

**Still missing:** “keep working while an older SDK value is still cached” does not state the transition rule. This matters beyond one old sidebar observation: the next setting command rereads cached settings (`management.rs:232–243`), and the SDK success projection starts from that cached snapshot (`room_operations.rs:307–308,344`). A history save immediately following an access save can therefore carry an old access value, or vice versa.

**Smallest correction:** explicitly extend the existing transition-based reconciliation, preserving unrelated accepted local fields until their authoritative observation advances. Define how successful local access changes reach the shared access projection and how external changes replace them. Do not add another subscription or retry owner.

**Acceptance:** hold the local echo, deliver an unchanged old observation, save the other property, then deliver echoes/external changes. Assert no rollback in settings, badge facts or combined outcomes. The planned sequential second-client test alone does not catch this.

---

### 7. IMPORTANT — **Partially resolved:** full-policy comparison is fixed; unavailable-edit failures remain open

**Revision 2:** plan:147–149 explicitly compares:

> “the full policy value (rule + allow targets)”

This genuinely resolves the scalar comparison defect.

**Source:** `RoomInfoPanel.tsx:120–128,195–227` currently stores and compares a string target; `SettingsPropertyCard.tsx:348–353,374–379` uses scalar equality to suppress Save.

Plan:179–181 also promises typed invalid/unsupported failures. That is an improvement over `crates/koushi-core/src/room/operations.rs:225–228`, which maps invalid settings to SDK failure, and `RoomInfoPanel.tsx:728–730`, which displays generic text.

**Still open:** the requested **unavailable-policy** discriminator is absent. The renderer must distinguish “unsupported current condition” from “current policy could not be verified,” not suggest a malformed selection in both cases.

**Smallest correction:** add only the narrow policy-edit failure kinds required by these cases, through both protocol failure and reducer property-status mapping. Use one canonical target-set representation; test reordered/duplicated server targets as well as a genuinely changed target.

---

### 8. IMPORTANT — **Resolved:** the actual badge/header/summary consumers are now covered

**Revision 2:** plan:202–209 explicitly names `panes.tsx`, both literal-label tooltip branches, Room Info’s binary summary and DM special case, confirmed summary values, navigation, and:

> “the `knockRestricted` request route keeps its own badge”

**Source:**

- `apps/desktop/src/components/Shell.tsx:1409–1414`
- `apps/desktop/src/components/panes.tsx:1029–1046`
- `apps/desktop/src/components/RoomInfoPanel.tsx:294–322,737–757`
- `apps/desktop/src/domain/accessCondition.ts:101–125`

These are the real consumers and special cases. Plan:242–244 also covers the requested contexts and tooltip focus. This item is resolved as a design commitment.

---

### 9. IMPORTANT — **Resolved:** verified type, safe naming and target cardinality are separate

**Revision 2:** plan:125–131 now keeps `kind` separate from optional `display_name`, counts all distinct targets before filtering, and keeps unnamed identities inside Rust.

**Source:** `vendor/matrix-rust-sdk/crates/matrix-sdk-base/src/room/mod.rs:222–236,325–335` exposes room type/create information, including the missing/redacted caveat. Current unsafe fallback is in `crates/koushi-sdk/src/room_projection.rs:2318–2324`; current name filtering is in `crates/koushi-state/src/sidebar.rs:648–663`.

The revised contract is correct.

**Implementation caution:** `create_content().is_some()` alone is not proof of an unredacted original type; the SDK documents defaulted fields for older redacted create events. Verify event availability/redaction when needed rather than treating every non-Space result as positively verified ordinary Room.

**Acceptance:** include one named Space plus one unnamed target, duplicate membership entries for one Space, and a redacted/missing create event.

---

### 10. IMPORTANT — **Partially resolved:** the mirror list improved but still omits requested integration points

**Revision 2:** plan:264–278 now includes SDK write/success projection, Core conversion files, protocol events, `coreEvents.generated.json`, forwarder fixtures and the canon. The delete list correctly excludes duplicate Tauri DTOs.

**Source:** the listed SDK and Core changes are necessary:

- `crates/koushi-sdk/src/room_operations.rs:260–266,329–344`
- `crates/koushi-sdk/src/room_projection.rs:1919–1920,2037–2051`
- `crates/koushi-core/src/room/management.rs:155–167`
- `crates/koushi-protocol/src/event/room.rs:124–130`
- `apps/desktop/src-tauri/src/core_event_forwarder/tests.rs:1172–1214`

**Still omitted from the actual mirror inventory:**

- **`apps/desktop/src/App.tsx`**—mentioned in “current state,” but not in the update checklist.
- **Command contract tests**, specifically `apps/desktop/src-tauri/src/commands/contracts.rs:1311–1343,1574–1609`.
- Core connection and frontend transport/API wiring for the new preview route.

**Smallest correction:** link the existing exhaustive inventory in `docs/agents/state-ownership.md:55–102`, then explicitly name these production integration points. Populate restricted-policy fixtures with real nonempty content, not `None`/empty placeholders.

Nested settings already travel as `RoomManagementState` (`apps/desktop/src-tauri/src/dto.rs:316`); no duplicate settings DTO is justified.

---

### 11. IMPORTANT — **Partially resolved:** production assertions are better, but the documented command does not select Core QA

**Revision 2:** plan:248–257 now requires surface payload assertions, synced allow/history readback, second-client changes and narrow/short pseudo-locales. Those resolve significant round-1 gaps.

**Critical remaining production-path gap:** the landing command at plan:286 is:

> `qa:headless-local -- --server=both`

It lacks **`--core`**. `scripts/desktop-headless-local-qa.mjs:101–104` sets:

> `const runCoreQa = args.has("--core");`

The package script supplies only `--run` (`apps/desktop/package.json:20`). Therefore this exact command does not establish the promised Core production-path evidence.

The plan also still does not select the scenario that will enforce the new assertions. The minimal existing home is `room_management`: `crates/koushi-qa/src/bin/headless_core_qa/scenarios/rooms.rs:925–927` already invokes focused subchecks. Its current `space_access` check observes scalar join rules, not restricted allow content (`rooms/space_access.rs:102–113,136–155`).

**Smallest correction:** extend that existing scenario and give the actual focused command, for example:

```text
npm --prefix apps/desktop run qa:headless-local -- \
  --server=both --core --scenario=room_management --timeout-ms=240000
```

Update its evidence-token contract rather than adding a new QA framework.

**Acceptance still incomplete:**

- unavailable allow data, alongside the four content cases;
- all four history policies through **both production submissions**;
- failed directory read and join-rule/publication independence;
- selected allow Space different from attachment and no permission to modify that Space;
- local-write/stale-cache ordering from item 6;
- second-client **history** change through the live production path;
- cancellation, failed save, retry, read-only and keyboard associations;
- explicit encryption/key limitations and non-retroactivity.

“Two parent attachments with one allow Space” at plan:227–228 needs a fixture sequence: creation currently accepts only one `parent_space` (`command/room.rs:24–26`). Create with one attachment, add the second through the existing link command, then verify that the allow content remains exactly the selected target. Do not extend creation to plural parents merely to satisfy the test.

---

### 12. IMPORTANT — **Partially resolved:** mobile correction is genuine; Element Web is still inaccurately described

**Revision 2:** plan:91–100 correctly says Android/iOS have restricted editing. Plan:106–111 correctly records screen-level versus property Save, folded history values, Android directory adjustment and stricter condition handling.

**Independently inspected source:**

- Android `SecurityAndPrivacyView.kt:234–265`: Space-member and combined request/member choices.
- Android `SecurityAndPrivacyPresenter.kt:275–341,475–506`: selection and full restricted write mapping; history folding.
- Android presenter `:414–431`: automatic directory adjustment.
- iOS `SecurityAndPrivacyScreenViewModel.swift:291–325,339–377`: target selection, restricted writes and history folding.
- iOS `View/SecurityAndPrivacyScreen.swift:44–87`: visible choice rows.

These files are in the available `/tmp/exa` and `/tmp/exi` reference trees.

**But plan:101–104 says Web has two groups “each a select.” That is false in the available Web source.** Under `/home/shinaoka/projects/Matrix/reference-repos/element-web/apps/web/`:

- `src/components/views/settings/JoinRuleSettings.tsx:416–426` renders **`StyledRadioGroup`**.
- `src/components/views/settings/tabs/room/SecurityRoomSettingsTab.tsx:475–481` also renders **`StyledRadioGroup`**.
- `JoinRuleSettings.tsx:93–109,237–263,354–410` implements a real restricted-target management dialog and membership write.
- `SecurityRoomSettingsTab.tsx:227–247` writes history on selection; `:363–411` can also change WorldReadable history while changing public access to non-public.

**Smallest correction:** cite these implementation paths and inspected revisions; replace the “select” claim and record Web’s immediate-commit and coupled-history behavior as intentional divergences. The available Web checkout is at `48e4bce28e46b0161dbc8ca6b9dd2a3c2867d0d6`. The plan’s abbreviated mobile filenames still need reproducible revision references.

---

### 13. MINOR — **Resolved:** canon amendment and mutation wording are corrected

**Revision 2:** plan:35–37 and :277 explicitly include the settable-rule canon amendment. Plan:51–54 correctly calls the mutation an ordinary state event.

**Source:** `docs/architecture/state-machine.md:3758–3776` contains both the old settable-rule restriction and reconciliation/cache rules. `vendor/matrix-rust-sdk/crates/matrix-sdk/src/room/privacy_settings.rs:131–138` sends an ordinary state-event request.

The “delayed-state” wording is gone. Amend the **reconciliation and success-projection sections too**, not only the admitted rule list.

## Additional revision-2 risks

### N1. IMPORTANT — React-owned selected policy/targets conflict with the current ownership canon

Plan:115–116 states:

> “React owns the uncommitted selection”

But `REPOSITORY_RULES.md:139–143` requires state affecting Matrix command shape or **selected target** to be modelled first as serializable Rust state/events. `docs/architecture/overview.md:185–192` likewise places room-management semantics in Rust.

The existing local scalar draft in `SettingsPropertyCard.tsx:320–321` does not authorize expanding that pattern into a target-selection owner.

**Smallest alternative:** explicitly reconcile this contract before implementation. Keep DOM/focus and raw input drafts local, but model policy/target selection and its semantics through the existing Rust workflow boundary—or approve a narrowly specified canon exception for a stateless, Rust-validated draft contract. Do not quietly create React-owned target eligibility or success semantics, and do not solve this with an independent outcome slice.

### N2. MINOR — Acceptance accidentally requires Save to be disabled when there is a change

Plan:238–240 says:

> “Save disabled on a real change including an allow-list-only change”

That is the opposite of the required behavior and existing guard (`SettingsPropertyCard.tsx:374–379`).

Replace with: **Save enabled for a valid real change; disabled when unchanged, pending, forbidden or invalid.** Include an allow-list-only change.

### N3. IMPORTANT — Detailed encrypted-history outcomes remain underspecified

The resolver vocabulary at plan:132–136 lists “who can read which history” and “encrypted,” but no explicit rule prevents it from implying that eligible users necessarily possess old decryption keys. The acceptance list does not assert these explanations.

Current Room Info separately renders Shared-encrypted and non-retroactivity notes (`RoomInfoPanel.tsx:468–481`); replacing that editor can silently remove them while all listed submission/readback checks pass.

**Smallest alternative:** retain those limitations in the shared Rust outcome vocabulary, explicitly separating server history eligibility from key availability. Assert all four policy explanations, encrypted limitations, and that changing policy does not revoke already obtained events/keys. No key-repair mechanism or crypto fork is required.

## Delete list

Both requested ponytail skill files were read. The round-1 deletions are largely resolved as scope commitments; keep them enforced during implementation.

| Round-1 item | Status and one-line over-engineering instruction |
|---|---|
| **14 — IMPORTANT** | **Resolved.** plan:139,291–292: `delete:` no independent persistent outcome slice; reuse existing access/settings projection plus the pure preview resolver (`runtime/connection.rs:1125–1151` is the stateless precedent). |
| **15 — MINOR** | **Resolved.** plan:293–294: `reuse:` no new write mechanism or SDK fork; use `RoomPrivacySettings::update_join_rule` (`privacy_settings.rs:131–138`). |
| **16 — IMPORTANT** | **Resolved in scope; safety detail remains under item 1.** plan:123–124,295–296: `shrink:` no arbitrary unsupported-condition round-trip; reject using current completeness facts. |
| **17 — MINOR** | **Resolved.** plan:27,297–298: `yagni:` no ordinary-room target picker; preserve existing conditions or reject their rewrite rather than filtering them (`room_projection.rs:2001–2023` shows why IDs must not simply disappear). |
| **18 — MINOR** | **Partially resolved.** plan:299–300 conflicts with the still-open creation expansion at :306–308: `yagni:` defer new `knock_restricted` creation and capability frameworks; preserve existing knock-restricted display/request behavior, and implement only the agreed creation routes. |
| **19 — MINOR** | **Resolved.** plan:185–190,301: `reuse:` no second property-card/status/focus framework; retain `SettingsPropertyCard` and existing edit-focus behavior (`SettingsPropertyCard.tsx:322–344`). |
| **20 — MINOR** | **Resolved.** plan:302: `reuse:` no duplicate Tauri settings DTOs or unrelated registration edits; nested settings already pass through `RoomManagementState` (`dto.rs:316`); register only the genuinely new preview command. |

Additional cuts:

- **MINOR — plan:227–228:** `reuse:` no plural-parent creation API merely for QA; add the second attachment with existing `SetSpaceChild`.
- **MINOR — plan:309–311:** `delete:` no permission-model expansion by default; retain the explicitly selected aggregate history gate.
- **IMPORTANT — plan:172–178:** `reuse:` no second policy subscription, polling loop or cache-repair owner; extend the existing observation/reconciliation path.

**Net:** avoided implementation surfaces, not a measurable line reduction before a code diff exists.

## What I did not check

- No files were edited; no builds, tests, QA scenarios, browser sessions or homeserver requests were run.
- No shell commands were executed. Branch/commit identification came from read-only Git metadata, not `git log`; ancestry and the diff against `origin/main` were not verified.
- I did not verify submodule gitlink identity, or the Git revisions of the Android/iOS reference trees.
- I inspected Element Web implementation, but not the claimed Playwright permission spec.
- I did not verify native-window behavior, screen-reader behavior, actual local-server support for every room-version/policy combination, or decryption outcomes.
- This is a design review of relevant source paths and selected tests, not an implementation audit or exhaustive repository review.
