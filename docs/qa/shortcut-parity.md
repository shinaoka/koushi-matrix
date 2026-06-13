# Shortcut Parity

This table records the current desktop shortcut surface in `apps/desktop/src/domain/shortcuts.ts`.
It uses synthetic application state only.

## Implemented

| Action | Shortcut | Notes |
| --- | --- | --- |
| Send message | `Enter` | Sends the composer draft when it is non-empty. |
| New line | `Shift+Enter` | Inserts a newline in the composer. |
| Cancel reply or edit | `Esc` | Closes the local composer affordance. |
| Search in room | `Ctrl/Cmd+F` | Moves focus to the search box and narrows scope to the active room. |
| Find rooms | `Ctrl/Cmd+K` | Moves focus to the search box and restores all-room scope. |
| Toggle right panel | `Ctrl/Cmd+.` | Opens or closes the contextual right panel. |
| Keyboard settings | `Ctrl/Cmd+/` | Opens the keyboard settings panel. |
| User settings | `Cmd+,` on macOS | Opens the user settings panel. |
| Go home | `Ctrl+Alt+H` | Current cross-platform row for the home action. |
| Select room | `Enter` | Selects the focused room list entry. |
| Previous room | `ArrowUp` | Moves within the room list. |
| Next room | `ArrowDown` | Moves within the room list. |
| Close dialog or menu | `Esc` | Used by overlay dismissal paths. |
| Activate focused control | `Enter` | Standard accessibility activation path. |

## Known Deviations

| Area | Status | Notes |
| --- | --- | --- |
| Composer formatting | Deferred | Bold, italic, link, and code shortcuts are present in the registry but the corresponding formatting actions are not implemented yet. |
| Room timeline navigation | Deferred | Page up/down, oldest unread, first/latest message, and similar timeline shortcuts remain placeholders. |
| Room list expansion | Deferred | Collapsing and expanding room list sections is not wired yet. |
| Unread-room navigation | Deferred | Previous/next unread room shortcuts are reserved for later phase work. |
| Autocomplete | Deferred | Escape and arrow navigation inside autocomplete are not active because the feature is not present. |
| Call controls | Deferred | Microphone and other call shortcuts stay deferred with the call feature set. |

## Evidence

- Shortcut registration and parity are covered by `apps/desktop/src/domain/shortcuts.test.ts`.
- Native menu accelerators are checked by the same registry audit.
- Keyboard reachability of the shell is covered by `apps/desktop/e2e/desktop-shell-a11y.spec.ts`.
