# #2007: clicking Reply focuses the composer

## The bug

Clicking **Reply** on a message set the Rust-owned reply target, so the reply
banner appeared above the composer, but keyboard focus stayed on the timeline row.
Typing right after the click went nowhere: the user had to click into the composer
first. `apps/desktop/src/components/composer.tsx` focused its editor after its own
mutations (toolbar inserts, send-key routing) and never when a reply target
arrived, in the room composer or in the thread pane.

## The rule

The composer watches the reply target it already receives as `composerMode`
(`{ kind: "reply", in_reply_to_event_id }`, projected from
`snapshot.state.ui.timeline.composer.mode`). A transition from no reply to a reply
focuses that surface's editor and puts the caret after the existing draft, which is
what Element does.

Five cases must not move focus, and each is pinned by a test:

- **Cancel** clears the target (`reply` → `plain`), which is not a transition into
  a reply.
- **A parent snapshot that names the target already shown** is not another Reply,
  even when the same snapshot also changes the draft.
- **Room or thread switch** arrives with that draft's own target already set, so the
  effect resets its baseline when `draftKey` changes instead of treating the
  arriving target as a click.
- **An assistant draft staged through MCP `draft_message`** can carry
  `reply_to_event_id` (#1906), and `stage_assistant_draft` writes the marker and the
  reply target in one commit, so a `revision` that changed in this transition skips
  the focus. A marker that predates the transition must not: it survives user edits
  for the rest of the draft's life, and the user's own Reply still has to focus.
- **An editor that already has focus** (or is mid-IME-composition) is left alone;
  the check is `sectionRef` containment plus `isComposing()` on the editor handle.
  A composition in this editor implies focus, but a browser that keeps one alive
  across a focus move must not have it interrupted, so both checks stay.

The caret write moves the composer's own `documentSelection` too: the editor writes
the DOM selection only, and the mention popup and the Rust mention-query effect are
computed from the React copy, so a stale offset would leave a popup open for a query
the caret has left.

Accepted limit: a Reply naming the message already shown never refocuses, because
the target is unchanged. That covers the user's second click on that message, and
also their first click when the target came from an assistant's staged reply draft. Re-focusing on every parent snapshot instead would
pull the user back out of whatever they clicked next, so the fix keeps focus
attributable to a target change; the user's next click into the composer is one
click, where the reported bug stranded them for every reply.

The effect follows the existing focus idiom in this file: `requestAnimationFrame`,
then `focus()` and a selection write on the editor handle, reading the draft length
from a ref because the frame runs after the commit that scheduled it. No new focus
mechanism and no new prop.

## Verification

```bash
npm --prefix apps/desktop run typecheck                  # clean
npm --prefix apps/desktop run lint                       # clean
npm --prefix apps/desktop test -- --run                  # 208 files, 2612 tests passed
```

RED, with the source change stashed and the new tests kept:

```
× focuses the room composer when a reply target is set
× focuses the thread composer when a reply target is set
× leaves focus where it is when the reply is cancelled
  AssertionError: expected <body>… to be <div …>  (the editor never took focus)
```

Each guard is pinned by deleting it and watching one test fail:

| Guard removed | Test that fails |
| --- | --- |
| `previous.draftKey !== draftKey` | keeps focus off a draft that changes room with its own reply target |
| reply target identity | leaves focus where it is when the reply is cancelled |
| assistant marker revision | does not take focus for an assistant draft staged through MCP |
| `sectionRef` containment | leaves the caret alone when the editor already has focus |
| `isComposing()` | does not interrupt an editor that is mid-composition |
| `setDocumentSelection(caret)` | moves the mention query to the caret it writes |

The "must not move focus" tests park focus on a button appended to `document.body`
rather than on a composer control, and they wait one animation frame before
asserting, so the containment guard is not what silently makes them pass and a
stolen focus cannot hide behind the frame.

The tests use `window.document.activeElement`, because the composer's own `document`
prop shadows the DOM document inside that component.

## Canon consulted

`docs/architecture/state-machine.md` for the Rust-owned composer mode (the renderer
dispatches `setComposerReplyTarget` and renders the projected mode; it does not
track reply state itself), and `docs/policies/engineering-rules.md` for the
IME-safe text-entry contract the focused editor already implements.
