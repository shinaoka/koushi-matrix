# Cmd+B / Cmd+I in the composer (#1949)

The Bold and Italic buttons inserted their inline markdown, but the
Element-compatible keys behind them did nothing. The composer key handler
(`onComposerKeyDown`, `apps/desktop/src/components/composer.tsx`) handled the
macOS Emacs bindings, the mention autocomplete, Tab-to-send, and then handed
Enter/Escape to `resolveComposerKeyAction` — `shouldResolveComposerKeyEvent` only
returns true for those two keys, so every other key fell through untouched.
`domain/shortcuts.ts` nonetheless listed **Bold — Ctrl/Cmd B** and
**Italics — Ctrl/Cmd I** with Element parity, so the settings page advertised a
shortcut that did not exist.

## Change

- `onComposerKeyDown` gains the format pair, before the autocomplete branch:
  primary modifier (`Cmd` on macOS, `Ctrl` elsewhere) + `B`/`I`, no other
  modifier. It calls `applyInlineFormat(kind)`, the function the toolbar buttons
  now call too, so the pairs (`**` → bold, `_` → italic, and their placeholder
  text) live in one module-level `INLINE_FORMAT` table and both entry points end
  in `applyInlineMarkdown`. Nothing is implemented twice.
- The modifier split is not cosmetic: on macOS `Ctrl+B` is AppKit's
  backward-char, which `macEmacsActionFromEvent` owns, and
  `shortcutIdForKeyboardEvent` already states the same rule for app shortcuts.
  So macOS takes `metaKey && !ctrlKey`, everywhere else `ctrlKey`.
- Two refusals, both required by the reported shape:
  - `event.nativeEvent.isComposing` — a composition owns the keyboard, so the
    key belongs to the IME candidate window.
  - `canEdit` — the read-only lock from #1941. `commitEditorMutation` already
    refuses writes while locked, so this is belt-and-braces: the shortcut does
    not swallow the key either.
- `docs/help/messaging.md` gains a "Format a message" section (the guide had no
  formatting text at all). The in-app shortcut table already lists both rows, so
  nothing needed adding there, and its `implemented: false` flags are left alone:
  that flag feeds `shortcutActionFromMenuPayload`, i.e. app-level action dispatch,
  and inline formatting is a composer-local edit with no app action behind it.
  Flipping it would open a menu-payload path to an unhandled action.

Selection behaviour is inherited, not re-implemented: `applyInlineMarkdown` reads
the editor's selection, wraps it, and falls back to the placeholder pair at the
caret — the same code the buttons use.

## Evidence

- `composer.test.tsx`, new `describe("composer format shortcuts (#1949)")`, four
  cases: the keyboard result equals the toolbar result for bold and italic
  (`**world**`, `_world_`) and `defaultPrevented` is true; the placeholder pair at
  the caret with no selection (`world**bold**`); a keydown with
  `isComposing: true` is neither prevented nor written; a locked composer writes
  nothing. The first two are **RED before the wiring** (`defaultPrevented` false,
  text unchanged `world`); the last two can only fail after a wrong
  implementation exists, since without the feature nothing happens at all.
- `e2e/composer-format-shortcuts.spec.ts` (new, Chromium DOM tier): types
  `world`, selects all, `ControlOrMeta+B` → `**world**`, selects all,
  `ControlOrMeta+I` → `_**world**_`, then the toolbar's Bold on the same
  selection → `**_**world**_**`, pinning the two paths to one result.
  **RED before the wiring** (`Expected "**world**", Received "world"`).
  `ControlOrMeta` sends Cmd on macOS and Ctrl elsewhere, so the hosted Linux lane
  exercises the other modifier than this machine does; the jsdom unit test covers
  the Ctrl path (`navigator.platform` is empty there).
- `npm run typecheck`, `npm run lint` (no issues), `npm run test -- --run`
  (206 files, 2540 tests), `node scripts/check-agents-docs.mjs`.

## Not covered

The inline message editor (`timeline.editBody`) is not a `Composer` and keeps
whatever shortcuts it has; #1949 is about the message composer, and the thread
composer and the scheduled-send panel editor share the component fixed here.
