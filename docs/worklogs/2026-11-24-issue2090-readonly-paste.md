# Refuse paste and cut in a read-only composer (2026-11-24, #2090)

`contentEditable={false}` stopped typing but not the editor's own clipboard
handlers. `handlePaste` and `handleCut` in
`apps/desktop/src/components/ImeTextControl.tsx` read the DOM selection, called
`event.preventDefault()` and then `publish(...)`. `publish` is the ordinary
document-change route, so it called `onDocumentChange`, the composer took the
new document and the draft persisted: a word selected in a locked composer was
replaced by the clipboard and saved as the room's draft.

Canon: `docs/architecture/overview.md` (React renders Rust DTOs and dispatches
typed commands; the composer document is renderer-owned until the draft save),
`docs/architecture/state-machine.md` (`SendBlocked` keeps the composer readable
and refuses the write surfaces), `docs/policies/engineering-rules.md` (a guard
needs a reproduced failure or a named invariant), and `docs/agents/qa-lanes.md`
(entering the WebKit `testMatch` needs a reason in the config comment).

## Change

- `publish`, `publishHistory` and `publishDom` refuse to write while the editor
  is read-only. These are the three write funnels, so the invariant "a read-only
  editor never publishes a document" now holds for every handler rather than for
  the two that were patched, including `beforeinput` deletes and the
  `input`/`compositionend` DOM read-back.
- `handlePaste` cancels the engine's default; the `publish` below it refuses, so
  neither a selected nor a bare caret paste can commit.
- `handleCut` additionally returns before copying the range, so a read-only cut
  does not overwrite the clipboard either.
- `handleCopy` is untouched: selecting and copying in a locked composer still
  work, which is what a reader of a read-only draft needs.
- No guard was added for a drop: the composer section's `onAttachmentDrop`
  already calls `preventDefault()` unconditionally, and re-cancelling it on the
  editor would be a second path for one refusal (`engineering-rules.md`,
  Design Simplicity). The end-to-end case pins that refusal instead.
- No per-event guard was added to `handleBeforeInput` either: `beforeinput` is
  dispatched at an editing host, and a `contentEditable={false}` div is not one,
  so the reported defect has no reproduction there. The funnel guard covers it
  anyway, which is why the unit case for a delete over a selection passes without
  a dedicated branch.
- `playwright.config.ts`: `readable-provisional-shell.spec.ts` joins the WebKit
  project. Whether a `paste` event reaches a non-editable element is the
  webview's own decision, and the report came from WebKitGTK.

## Verification

- `npm --prefix apps/desktop run typecheck` — clean.
- `npm --prefix apps/desktop run lint` — clean.
- `npm --prefix apps/desktop run test -- --run` — 216 files / 2722 tests passed.
- `npx playwright test e2e/readable-provisional-shell.spec.ts
  --project=chromium --project=webkit --repeat-each=2` — 40 passed.
- RED against the pre-change editor (`098875ff`), both engines:
  `Expected: "Draft saved before the lock"` / `Received: "Draft saved Clipboard
  text the lock"`. With the DOM-text assertion removed, the same build reports
  `set_composer_draft` invoked once and the saved draft rewritten, so the
  debounced draft check is load-bearing rather than timing-dependent.

Deviations, recorded:

- The end-to-end case waits 800 ms before asserting that no draft save was
  dispatched. `docs/agents/verification.md` asks for assertions on state rather
  than fixed sleeps, and every other assertion here is state-based; a save that
  must *not* happen has no settled signal to poll, and the composer debounces
  the save by 350 ms (`App.tsx:5710`), so the absence is observed after a
  bounded wait instead.

Not checked: the real Tauri webview. The e2e dispatches the clipboard event at
the editor; the guard covers any `paste`/`cut` that reaches it, but how each
webview routes a paste to a non-editable element is engine behaviour that no
headless run here reproduces.
