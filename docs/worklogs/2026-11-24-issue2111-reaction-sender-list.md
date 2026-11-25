# Everyone who reacted is reachable (#2111)

Reported: "リアクションした全員が見られない" — the chip tooltip named three people and
then said "N more", and no surface listed the rest.

The truncation was not a renderer choice: Core projected `sender_preview` with
`senders.keys().take(3)` in both the room and the thread projection, so the names
never reached the renderer at all.

## What changed

- `REACTION_SENDER_PREVIEW_LIMIT = 50` in `koushi-core`, with the reason in the
  doc comment: every name travels in each row's payload, and a row is re-sent on
  every timeline update, so the preview is bounded on purpose. Both projections
  use the one constant. `ReactionGroup.count` stays the true total — a list is
  never presented as complete: the renderer derives "N more" from
  `count - sender_preview.len()`.
- The tooltip names all of them, wraps, and scrolls
  (`max-block-size: min(40vh, 220px)`, `overflow-y: auto`, `pointer-events: auto`
  while visible) instead of clipping after three. The `uiListFormat` conjunction
  is unchanged.
- A right-click (or a 500 ms hold where there is no right button) on the chip
  opens a fixed list titled "Reacted with {key}". It is the #2098 read-receipt
  popover, extracted to `PersonListPopover` and used by both:
  `ReceiptReaders` keeps its hover semantics, the reactor list dismisses on an
  outside press, takes the keyboard focus on open, and returns focus to the chip
  when it closes. Rows are avatar + display name, the signed-in reader is marked
  with `people.you`, a row opens that person's profile, and ↑/↓/Home/End/Page
  move a focus that is always visible.
- A chip click is untouched: it still toggles, keeps `aria-pressed`, and reads
  `reacted_by_me` / `my_reaction_event_id` as before. The chips sit in a
  `.reaction-pill-shell` that owns the right-click and hold handling, so a
  disabled read-only pill (#2066) can still be asked who reacted.

## Not changed on purpose

- No attempt to list past the preview: the popover reports the same "N more" the
  tooltip does rather than presenting a partial list as complete.
- `sender_preview` is not paged or lazily loaded; the cap is the whole mechanism.

## Evidence

- Core: `reaction_groups_name_every_sender_up_to_the_preview_limit`,
  `reaction_groups_stop_the_named_preview_at_the_limit_and_keep_the_full_count`.
- Renderer: `ReactionSenders.test.tsx` (names every reactor, marks self, opens a
  profile, roving focus, keeps the dialog reachable with an empty preview),
  `TimelineView.rendering.test.tsx` (five reactors produce five names; a
  right-click opens the list while a click still sends the reaction).
- Headless: `e2e/timeline-reaction-senders.spec.ts` (tooltip names seven and
  scrolls at fifty, right-click list + Escape + toggle, keyboard movement and
  profile open). `e2e/receipt-popup-layout.spec.ts` still passes, so the
  extracted part kept the receipts' measured width and height.

## Independent review (glm/ollama-cloud, post mode)

Fifteen of its eighteen findings were reproduced against the source and fixed
here; the ones that mattered:

- The tooltip was clipped. `.message-list { overflow: hidden }` (#2078) cuts an
  absolutely positioned box above a chip near the pane's top edge, and its own
  scroll could not reveal the cut rows. The box now flips below the chip when the
  measured height does not fit above it (`placeReactionTooltip`), and the e2e case
  asserts real geometry instead of text.
- A group that lost its last reactor left `senderListKey` set, so the list
  re-opened by itself when the group came back and the next click was swallowed by
  the long-press guard. The key is dropped when the group disappears.
- Closing the list always focused the chip: opening a profile from a row pulled
  the focus back into the message list, and a right-click from the composer did
  not return the caret. `onClose(false)` on activation, and the focus captured
  when the list opened is restored when it is still usable.
- `focusOnOpen` could not enter an empty list (a group whose names Core did not
  send), leaving Escape unreachable; the dialog itself is now the focus fallback
  and carries `tabIndex={-1}`.
- A roving `tabIndex` whose remembered row had gone left the list with no tab
  stop; the first row takes it instead.
- `End`/`PageDown` walked past the last row on a list that cannot page; movement
  now ends at the rows this instance holds.
- A touch hold that slid off the chip still opened the list (implicit pointer
  capture means no `pointerleave`): a drift past 8px cancels it. A second
  right-click on the same chip now closes what it opened.
- The shared identity map dropped profiles without a display name, so their
  avatars were never drawn.
- The `PersonListPopover` refactor had silently moved the receipts' `align`/
  `placement` to the hook defaults; `end`/`above` are now pinned in the shared
  part and the never-passed props are gone. Dead `emptyText`, the duplicated
  `containsTarget` and `requestReaderWindowAt`'s unused return went with it.
- Wording: the cap bounds the names of **one reaction key** of a row, not a row,
  and the prose copies of "50" are gone in favour of the constant.
- The chip announces `aria-haspopup="dialog"`; a read-only chip still reaches the
  list through its shell, and a new e2e case covers exactly that.

Not taken: the receipts' pending-focus window guard (`readerWindowSequence`).
The rows it focuses are an append-only index into one request, so an index that
arrives later still names the same person; the guard would add state without a
case that can fail.

Still open by design: a **disabled** read-only chip is not focusable, so a
keyboard-only reader in a read-only room reaches neither the tooltip nor the
list. That is #2066's state, and changing it means giving the read-only pill a
focus stop — separate work, not a side effect of this issue.
