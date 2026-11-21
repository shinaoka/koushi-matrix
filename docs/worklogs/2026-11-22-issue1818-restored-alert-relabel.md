# Relabelling a restored native attention alert (#1818)

A native attention alert is identified by `alert_id`, minted in Rust where the
candidate is raised and deduped on by the webview (#1802, #1816). The identity
belongs to the **room** that holds the attention, and the reducer decided
sameness by comparing the room a raise targets with the room the published
payload targets. That payload is `#[serde(skip)]`, so after a restart the
comparison had nothing to compare: the next message in the same conversation
minted a second identity, and the webview — which dedupes on the identity alone
— showed a second banner for one unread conversation.

The issue recommended leaving it, on the grounds that fixing it means putting a
room into a summary that deliberately carries none. The decision was revisited
and the fix ordered, so this change pays that cost in the narrowest form it can.

## What changed

`NativeAttentionSummary` now carries `alert_room`, the **opaque handle** of the
room the published alert targets, next to `alert_id`:

- `native_attention_room_handle` is FNV-1a over the room id: fixed constants, so
  the same room yields the same handle in every process and build. `DefaultHasher`
  is explicitly not stable across processes, which is exactly what a restore needs,
  so `a_room_handle_is_stable_across_builds` pins one value — the constants are
  part of the stored format.
- It is a pseudonym, and the honest limit is in the comment: it is **unkeyed**, so
  anyone who already holds the room id can confirm the link, and it is stable
  across processes and accounts — the opposite of the keyed banner id #1975
  requires, and unavoidable here because a restart is exactly what it has to
  survive. What it does not do is disclose anything new (the snapshot already
  carries the room list), and it is compared only with itself: never rendered, put
  in a candidate, part of a diagnostic, or printed — `NativeAttentionSummary` has
  a redacting `Debug` so that last part is true by construction.
- The raise path compares handles instead of room strings, and stamps the handle
  whenever it publishes a candidate; a clear clears both. `#1806`'s
  carry-forward compares the same handle, so a submission after a restore keeps
  the identity too.
- An identity supplied by a caller keeps its own room: the handle follows the
  identity, and a submission that carries no payload keeps the handle it was
  given instead of dropping it. Without that, a submission shaped
  `alert_id: Some(n), notification: None` — the shape
  `crates/koushi-core-testkit/tests/runtime/runtime_settings.rs` builds — would
  publish an identity with no room and revert that alert to minting.
- The display-label refresh (`refresh_native_attention_candidate_display_projection`)
  now resolves the room from the handle first and the payload second, so a
  local-alias or profile change relabels the published candidate after a restore
  as well; it previously did nothing because the payload was gone.

The rule itself is unchanged — one identity per unread conversation until a
canonical clear — it now just holds across a restart as well.

## Tests

RED first: `a_restored_state_relabels_the_alert_it_published` failed on the old
code with `alert_id 2 != 1` (a restore, then a second foreign message in the same
room, minted a new identity). It passes with the handle in place.

- `a_restored_state_relabels_the_alert_it_published` — the relabel keeps the id,
  the count and body still move (the (a) case).
- `a_restored_state_mints_for_a_different_room` — the restore keeps one identity
  per conversation, not one for everything, and the `max(published) + 1` floor
  still holds (the (c) case).
- `a_submitted_identity_keeps_the_room_it_names` — an adopted identity owns its
  room.
- `a_restored_alert_ends_with_an_observation_that_raises_nothing` — the limit that
  remains, pinned so a later change to #1799's revival path is deliberate.
- `apps/desktop/src/app/useDesktopAttentionEffects.test.ts`: the pair of snapshots
  a restore produces shows one banner, which is only true while the dedupe key is
  the identity (the (b) case).

## Not done

- #1799's revival of a published candidate still reads the process-local payload,
  so an observation that raises nothing after a restore ends the alert. Extending
  that needs the room id rather than its handle, and it is a different behaviour
  change; the canon now states the limit.
- `alert_room` reaches the webview: the snapshot is a hand-maintained DTO
  (`apps/desktop/src-tauri/src/dto.rs`), and `state-ownership.md` requires a new
  `AppState` field to be mirrored there, so it is — in `types.ts` and
  `coreEvents.ts`, marked as not for React, because dedupe stays on `alert_id`.
  Keeping it out would mean blanking it in two projection functions and diverging
  the DTO from the state it mirrors; that is a decision for the privacy owner, not
  something to slip into this fix.
- FNV-1a is now implemented for the fifth time in this repository
  (`keyword_rules.rs`, `room_management.rs`, `room_history_export/layout.rs`, and
  here). Consolidating them is its own change; recorded rather than bundled.
