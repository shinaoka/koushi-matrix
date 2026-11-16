# App Settings → Account Settings cross-link (#1921)

The link at the bottom of the settings category list is disabled when the app scope
has no current session:

```tsx
disabled={settingsScope === "app" && !currentSession}
```

`AppSettingsDialog` is the only mount of App Settings in the shipped app
(`App.tsx:1114`), and it hard-coded `currentSession={null}` (`UserSettingsPanel.tsx:1047`)
even though the snapshot it receives holds the signed-in session. So in the shipped
app `!currentSession` was always true and the control could never be enabled — the
reporter's "does it turn blue later" check was the wrong question, since no state
could satisfy the gate. One page over, **App Settings → Search history** has its own
**Account Settings** button, wired through `onOpenAccountSettings` to the same scope
transition, and that one works. The guide (`docs/help/settings.md`) had been narrowed
on 2026-10-09 (`250b52a2`) to document the Account → App direction only, which
documented around the dead control instead of fixing it.

## Change

- `AppSettingsDialog` passes `currentSession={currentSavedSession(snapshot)}` — the
  same derivation the Account Settings mount in `rightPanel.tsx:742` uses, and the
  function the issue named. No new state, no new command, no new prop: the dialog
  already had the snapshot and `onSettingsScopeChange` was already wired to
  `setSettingsScope` in `App.tsx`.
- The gate itself is unchanged. It still refuses the switch when the app scope has
  no account, which is now a reachable state (App Settings can be open before or
  without a signed-in account) rather than a permanent one.
- Guide: `docs/help/settings.md` says either scope switches to the other with the
  button at the bottom of the category list, that the App Settings button needs a
  signed-in account, and that Search history keeps its own button. The duplicate
  Search history button stays: it is a contextual route to the same scope change,
  and the nav link is the permanent one.

What a real session does *not* change in app scope, checked because the prop is
shared with the account surface: the visible app categories are appearance,
preferences, keyboard, search, assistants and help (`UserSettingsPanel.tsx:269-273`),
and everything else that reads `currentSession` — the owner heading, the profile and
session detail rows, notifications, sign-out, device copy — lives in the hidden
account page or is gated on `settingsScope !== "app"`. `onRefreshCurrentSessionStatus`
is not passed by the dialog, so the session-status effect fires its no-op default, and
so does `AccountManagementSection`'s capabilities effect, whose handler is one of the
dialog's `noop`s; the notifications owner is the literal category id `notifications`,
which the app scope does not list. No new command reaches Rust.

## Evidence

- `UserSettingsPanel.test.tsx`, three new cases: the nav link is enabled and reads
  **Account Settings** with `readyDesktopSnapshotFixture()` (a signed-in snapshot);
  clicking it calls `onSettingsScopeChange("account")` exactly once; with
  `session: { kind: "signedOut" }` it stays disabled. The first two are **RED before
  the change** (`expected true to be false`, `expected "vi.fn()" to be called 1 times,
  but got 0 times`), matching the issue's headless probe.
- `e2e/settings-scope-cross-link.spec.ts` (new, Chromium DOM tier): opens App Settings
  from the top-bar gear, asserts the nav link is enabled, clicks it, and asserts the
  Account Settings dialog opened with the inverse link. **RED before the change**
  (`Expect "toBeEnabled"` received `disabled`). Chromium only — the defect is a React
  render gate, not engine behaviour, and the WebKit project is opt-in per spec.
- `npx playwright test e2e/settings-scope-cross-link.spec.ts e2e/emoji-picker.spec.ts
  e2e/search-crawler-settings.spec.ts e2e/mcp-assistants.spec.ts --project=chromium`:
  42 passed, the three neighbouring specs that drive the same dialog included.
- `npm run typecheck`, `npm run lint` (no issues), `npm run test -- --run`
  (206 files, 2529 tests), `node scripts/check-agents-docs.mjs`.

## Same class, not fixed here

`rg -n "settings-scope-switch" apps/desktop/src` matches one control
(`UserSettingsPanel.tsx:391-399`) plus its style rule, so this link had exactly one
mount site and is now live in both directions.

The dialog's other inert-looking wiring is deliberate and left alone: it passes a
dozen `noop` account handlers (`onSetDisplayName`, `onLogout`, …) because the account
page is rendered hidden in app scope and must not mutate an account from the
cross-account dialog. That is the "inert rendering is deliberate" exception the issue
describes; the alternative in the issue's rule — do not render the control — would
mean splitting the panel's markup by scope, which is a larger change than this defect.
The issue's broader suggestion (a lint rule or component-test convention that fails
when an interactive element is mounted with a no-op callback) is a cross-cutting
policy change and belongs in its own issue.
