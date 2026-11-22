# #2041 P3: the room-and-member-change preference lives in Core

P1 made a state event one muted information line and P2 grouped a burst into one
line with a count. A reader who does not care about profile or membership noise
still had no way to say so. This change adds **Show room and member changes**,
default on, and gives the answer to Rust.

## Why Core owns the switch

`timelineStore.ts` already records that Core owns the whole row-visibility
policy and that recomputing `is_hidden` in the renderer from one display
preference would be a second policy; that is exactly what resurrected bodyless
technical events before #1110. So the preference is a `koushi-state` setting,
carried on the existing display-policy path:

`SettingsChanged` → `DisplayPolicyUpdated` → `AccountMessage::DisplayPolicyChanged`
→ `TimelineManagerControl` → `TimelineActorControl` → `timeline_item_is_hidden`.

The actor recomputes visibility over the canonical window and publishes ordinary
`ItemsUpdated` diffs, so the renderer only ever honours `is_hidden` — the same
diffs it already gets when a message is redacted.

## Classifying the rows

`TimelineItem.notice_category` is the new datum: `profile` for a member's own
display name, avatar or room-profile change, `membership` for a join, leave,
invite or knock. Membership rows carry no localized notice key, so the
classification is made where the SDK content is still in hand
(`item_projection::notice_category`) rather than guessed later from a rendered
sentence.

The hideable set is explicit and everything else carries no category at all, so
the policy is one line and the exceptions live next to the classification. What
stays visible, and why: encryption and a room upgrade are room state, not a
membership or profile change; a ban or a kick is a moderation action, and
`Unbanned` joins them so a ban never looks permanent; a member event that changes
no membership (`MembershipChange::None`, e.g. a power level or a reason update) is
not a join or a leave, and hiding it would silence someone being made a moderator;
`Error` and `NotImplemented` are rows this build does not understand, and a row we
cannot classify must never be the one we silently drop.

`notice_category` is Core bookkeeping. The renderer learns the outcome as
`is_hidden`, and the commands it can send never carry a row back, so the field is
`#[serde(skip)]` and the checked-in core-event artifact is unchanged apart from
the `DisplayPolicyUpdated` field.

## Settings and persistence

`DisplaySettings.show_room_and_member_changes` sits beside `hide_redacted`,
serialized with the rest of the settings file, so it survives a restart with no
new store. `AppDisplaySettings` exposes it to the settings panel, and
`apply_patch` accepts it on the app scope. A settings file written before this
change has no key and deserializes to `true`, so an existing install keeps seeing
state changes.

The toggle is in **App Settings → Preferences → Timeline**, beside the
thread-placement switch, and its label and description are in all five locales.
Turning it off is one settings patch; the rows disappear because Core re-emits
them hidden, not because the panel did anything to the timeline.

## Evidence

- `crates/koushi-core/src/timeline/relay/display_policy_visibility_tests.rs`
  builds a room timeline with a profile change, a join, a ban, encryption, a room
  upgrade and a membership-update row, toggles the preference off through the
  actor message, and asserts that the profile and join rows arrive hidden while
  the other four stay visible; toggling it back on shows both again. The
  membership-update row fails against a catch-all classification, which is how the
  post-implementation review's finding was confirmed.
- `crates/koushi-core/src/timeline/item_projection/notice_category_tests.rs`
  walks every `MembershipChange` variant: ten are the class the preference hides;
  ban, unban, kick, kick-and-ban, no-change, error and not-implemented are not.
- `crates/koushi-state/tests/state/settings_state.rs` pins the patch, the stored
  value, the app projection and the legacy default.
- `crates/koushi-core/src/runtime/tests.rs` pins that the settings change emits
  `DisplayPolicyUpdated` carrying the new value.
- `apps/desktop/src/domain/timelineStore.test.ts` pins that the renderer keeps
  the rows it is handed, including a hidden profile notice beside a visible ban,
  so no renderer-side policy exists to disagree with Core.
- `apps/desktop/src/components/UserSettingsPanel.test.tsx` pins the switch in the
  Timeline section and the settings patch it emits.

## Consequences of hiding a row

The preference is part of the one row-visibility policy, so it inherits that
policy's reach: a hidden membership line also stops feeding the Activity list, the
same way a hidden redacted row does, and the read/unread divider moves to the next
row that is still shown because unread navigation skips hidden rows. Unread counts
themselves come from the server and do not change.

## Known limits

- Room configuration notices (name, topic, avatar, pinned events, join rules)
  stay visible whatever the switch says: the issue scopes the preference to
  profile and membership changes, and a room rename is not noise of the same
  kind.
- The preference cannot hide a membership line the SDK reports as a ban or a
  kick, and it cannot show one that it classifies as anything else. The
  classification is the SDK's, not a guess from the body text.
- Membership lines are still English sentences built at projection time; this
  change classifies them but does not localize them, so they read the same in
  every locale. Localizing them would change rendered text and belongs to its
  own change.
