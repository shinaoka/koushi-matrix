# Migrate to Koushi (#1871, Phase B)

Phase A shipped the engine and IPC (PR #1872): Element Desktop detection, the
DevTools-protocol export, the `0600` staging file, and
`AccountCommand::ImportMigratedRoomKeys`. There was no React surface, so nothing
in that change was reachable by a user. This is Phase B — the surface, the copy in
five locales, the browser test, and the docs.

## Route order is a product decision, and it changed

The issue's order put "verify with another device" first. The owner reversed that
while this work was in progress: device verification is the route that keeps
breaking (#1780, #1279, and the nightly-qa `user_verification` failures #1323,
#1534, #1169, #1462, #1319, #1305), so it is now **last**, with the stall named on
the screen and links back to the two deterministic routes. The order delivered is:

1. **Import an exported key file** — first, marked *most reliable*, and rendered
   with the primary button. Needs no other device and nothing delivered.
2. **Restore with your recovery key** — the section's own secret field calling
   `recover_secure_backup`, so it works without opening another panel.
3. **Import from Element Desktop on this computer** — behind the adapter flag.
4. **Verify with another device** — last, says it can stall, and carries buttons
   that move focus to route 1 and route 2 on the same screen.
5. **If nothing can be recovered** — states that the history stays unreadable, that
   a new backup protects future messages only, offers the new-backup action, and
   links `https://app.element.io/#/settings/security`.

## What changed, by layer

- **Adapter** (`apps/desktop/src-tauri/src/migration/`): detection is now
  cross-platform — macOS bundle, `%LOCALAPPDATA%\Programs\element-desktop`, and the
  four Linux package paths — and `running` is read on each platform (`pgrep` on
  macOS/Linux, `tasklist` on Windows). `describe()` is a pure function of the four
  facts, so the installed/running/unavailable combinations are tested off-platform.
  The DTO gained `import_supported`, `import_enabled`, and `unavailable_reason`.
  Staging refuses with `migration_element_desktop_import_disabled` unless
  `KOUSHI_ENABLE_ELEMENT_DESKTOP_IMPORT=1`, so a stale renderer cannot start a
  relaunch even though the UI hides the route.
- **Bridge**: `migrationDetectSources` and `migrationImportElementDesktop` on
  `DesktopApi`, their `TauriDesktopApi` methods, the #759 map entry, and the IPC
  mock's read-command set.
- **UI**: `MigrationSection` in the Encryption page, and the same component next to
  **Set up secure backup** on a first sign-in, where the gate offers it behind one
  button. The gate keeps its existing flow untouched: without the import callbacks
  it renders exactly as before.
- **One import path**: the key-file import moved out of `SecuritySection` into
  `RoomKeyImportForm`, which both the Security section and route 1 render. Same
  chooser, same passphrase modal, same status row (`room-key-import-state`), so the
  migration surface cannot drift from the feature it points at. Status labels moved
  to `KeyManagementStatus.tsx` for the same reason.

## Key material

Nothing new carries it and nothing prints it:

- Detection returns counts, booleans, and stable codes — no paths, no keys, no
  port. (Phase A's `profile_hint` and Element X's unread `unavailable_reason` were
  dropped for this: nothing rendered them, and a home-directory name is a path.)
- The recovery secret is only ever passed to `recover_secure_backup`; the field is
  cleared on the element as well as in React state, and a test asserts the submitted
  value is gone from the DOM.
- Element Desktop's export still goes through the `0600` staging file that Core
  deletes, with the startup sweep for interrupted imports (#1872).
- The Element import reports only `{staged_keys, staged_rooms}`; the panel prints
  those counts and nothing else.

## Verification

- Rust: `cargo test -p koushi-desktop --lib migration::` — 17 passed, including the
  four installed/running combinations, the flag's exact-value rule, the Windows
  install path (compared by components so it holds on any host), the Linux package
  order, `tasklist` parsing, and the fail-closed staging code.
- Frontend: `MigrationSection.test.tsx` 11 tests — route order and the *most
  reliable* marker, the file import going through the shared path, the recovery
  route, Element consent (cancel does nothing, confirm reports 12 keys / 3 rooms),
  the route hidden when the flag is off, detection for running/Element X/absent/
  failed, the verification route plus its two focus links, the nothing-recoverable
  copy with the Element Web URL, and the shared status row.
- Gate: `SessionVerificationGate.test.tsx` gained the first-sign-in offer, which
  proves both that the panel appears and that a gate without the callbacks is
  unchanged.
- Browser: `e2e/migrate-to-koushi.spec.ts`, chromium, 3 tests — measured route
  order, the flag-off variant, and the nothing-recoverable exit.
- `npm test -- --run` — 212 files / 2647 tests.
- Full chromium DOM tier — 689 passed.

## Review round

Independent review (GLM 5.3) found two blockers worth more than the copy this PR
is about, and both were real:

1. **The first-sign-in offer was unreachable.** `App.tsx` — the only product
   caller of `SessionVerificationGate` — never passed the two import callbacks, so
   the panel existed only inside a unit test that supplied them by hand. Since the
   gate owns the window exactly when nothing is readable, migration was
   unreachable precisely for the new device that needs it. Fixed in `App.tsx`, and
   `App.test.tsx` now pins the gate's render site passing both callbacks, so the
   omission fails a test rather than passing silently.
2. **Route 3 was offered with Element not installed.** The UI gated on
   `import_supported && import_enabled`, which say "this platform could" and "the
   flag is on" — not "the application is here". On macOS with the flag set and no
   Element, the screen offered the route directly above "No Element Desktop on
   this computer", and pressing it could only fail. The route now requires
   `installed`, and the test flips only `installed` so the old condition cannot
   come back green. Both blockers were confirmed RED against the old code before
   the fix.

Also fixed from the review:

- Route 2's failure line promised a diagnosis the command cannot give:
  `recover_secure_backup` returns when Core admits the request, and the outcome
  arrives later as the backup status. The copy now says the result appears there,
  and one honest line covers a request that never started (`migrateRequestFailed`)
  instead of two key-specific messages that could not fire.
- "Element is opened again afterwards" was false when Element was closed: Koushi
  starts it for the export and closes it again. The copy says that.
- Route 5's "Create a new secure backup" only scrolled; with a backup already set
  up there is no form there, so the button did nothing. The handler is now
  optional and omitted in that case, and the browser test asserts it is absent.
- The recovery secret is read from the element at submit instead of being copied
  into React state on every keystroke (`#927`).
- A dead `setSources({ ...before })` that pretended to re-detect after the import
  was deleted.
- The Element import answers with staged counts, so the IPC mock returns those
  instead of a bare admission that would make a future test read `undefined`. The
  review called this a latent test hazard; it was not latent. The DOM tier caught
  it: an unstubbed `migration_detect_sources` returned a truthy value without
  `element_desktop`, the panel threw on `sources.element_desktop.installed`, and
  every spec that opens App Settings timed out. The mock now answers detection
  with the full DTO shape unless a spec sets its own.
- Windows detection also checks `C:\Program Files\Element\Element.exe`. Flatpak
  and AppImage remain unfound, and the architecture note now says so rather than
  implying a definitive "not installed".
- The migration import row gets its own status hook: one page mounts two forms.
- zh-Hant used 通關語 for the key-file passphrase where the rest of the catalog
  says 密語, and both Chinese "restore" buttons read as nouns; fixed.

Rejected, with the reason:

- Deleting the second import form on the Encryption page (route 1 *is* the
  migration entry; the Security section keeps the reference one, and the hooks are
  now distinct) — the duplication is the price of the panel not inventing a second
  import path.
- Dropping the Element Desktop route to a link because "verify" is last: the owner
  asked for parity once it works, and it is flag-gated until then.
- The port-squat window at `127.0.0.1` (Phase A, documented) and the synchronous
  `pgrep`/`tasklist` inside the export path: real, unchanged by this PR, and
  recorded rather than widened here.

## Not done

- The CDP import has not been run against a real Element installation; the adapter's
  own fake-server tests cover the protocol and the flag, and the surface is hidden
  unless the flag is set. Promotion from "flagged" to "peer of route 1" needs one
  real round trip on a disposable account.
- Element X remains detected-only, unchanged from Phase A: its store is encrypted
  under a key in its own keychain.
