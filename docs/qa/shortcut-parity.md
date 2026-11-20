# Shortcut Parity

This table records the shortcut rows this QA pass has verified against
`apps/desktop/src/domain/shortcuts.ts`. That registry, not this page, is the complete
list; a row missing here is unverified, not unsupported. The tables use synthetic
application state only.

**Implemented** means the key produces the named action on the platform listed, in
whatever surface owns it — including one that only acts inside a focused surface, such
as the composer's inline formats.

**Not wired** means no handler acts on the key yet, either because its registry row has
no dispatch or, where the row says so, because the shortcut is not in the registry at
all. Each such row names the missing piece.

Neither word is the registry's own: `implemented: false` there means "no app-level
action for native-menu dispatch" (`apps/desktop/src/domain/shortcuts.ts:21-27`), and
`parity: "deferred"` is that row's parity value, which is what **App Settings →
Keyboard** renders — the page shows parity, not `implemented`, so a row the registry
marks `implemented: false` still shows as *same* there.

Last checked against `apps/desktop` at `99093529` (2026-11-20).

## Implemented

| Action | Shortcut | Notes |
| --- | --- | --- |
| Send message | `Enter` | Sends the composer draft when it is non-empty. |
| New line | `Shift+Enter` | Inserts a newline in the composer. |
| Bold | `Ctrl/Cmd+B` | Wraps the selection, or inserts the placeholder pair, exactly as the toolbar button does. |
| Italic | `Ctrl/Cmd+I` | Same contract as Bold, for the toolbar's Italic button. |
| Cancel reply or edit | `Esc` | Closes the local composer affordance; while the mention list is open it dismisses that first (`apps/desktop/src/components/composer.tsx:838-841`). |
| Search in room | `Ctrl/Cmd+F` | Moves focus to the search box and narrows scope to the active room. |
| Find rooms | `Ctrl/Cmd+K` | Moves focus to the search box and restores all-room scope. |
| Toggle right panel | `Ctrl/Cmd+.` | Opens or closes the contextual right panel. |
| Account Settings | `Cmd+,` on macOS | Opens the modal Account Settings dialog. |
| Go home | `Ctrl+Alt+H` | Current cross-platform row for the home action. |
| Select room | `Enter` | Selects the focused room list entry. |
| Previous room | `ArrowUp` | Moves within the room list. |
| Next room | `ArrowDown` | Moves within the room list. |
| Close dialog or menu | `Esc` | Used by overlay dismissal paths. |
| Activate focused control | `Enter` | Standard accessibility activation path. |

## Known Deviations

| Area | Status | Notes |
| --- | --- | --- |
| Composer link and code | Not wired | `formatLink` and `formatCode` (`apps/desktop/src/domain/shortcuts.ts:102-117`) have registry rows and keys but no keyboard handler: `inlineFormatFromKey` maps only `b` and `i` (`apps/desktop/src/components/composer.tsx:113-121`), so the composer formats them from the toolbar buttons alone. |
| Room timeline navigation | Not wired | Page up/down, oldest unread, first/latest message, and similar timeline keys are not in the registry and have no handler; only the "jump to bottom" button exists. |
| Room list sections | Not wired | Each section header is a toggle button that collapses and expands it (`apps/desktop/src/components/Shell.tsx:1613-1622`); the `ArrowLeft`/`ArrowRight` shortcuts are not wired to it (`collapseRoomListSection`/`expandRoomListSection`, `apps/desktop/src/domain/shortcuts.ts:208-221`). |
| Unread-room navigation | Not wired | Previous/next unread room shortcuts are reserved for later phase work. |
| Call controls | Not wired | Microphone and other call shortcuts stay deferred with the call feature set. |

## Evidence

- Shortcut registration and parity are covered by `apps/desktop/src/domain/shortcuts.test.ts`.
- Native menu accelerators are checked by the same registry audit.
- Keyboard reachability of the shell is covered by `apps/desktop/e2e/desktop-shell-a11y.spec.ts`.
- The composer's Bold and Italic shortcuts are pinned twice: `apps/desktop/src/components/composer.test.tsx:2205-2247` asserts they produce the toolbar's result, insert the placeholder pair at a bare caret, and stand aside for an IME composition; `apps/desktop/e2e/composer-format-shortcuts.spec.ts:20-38` drives Cmd/Ctrl+B and Cmd/Ctrl+I in the shell and compares them against the toolbar button (both from #1949, PR #1955).
- The user-facing promise is `docs/help/messaging.md:18-25`.
