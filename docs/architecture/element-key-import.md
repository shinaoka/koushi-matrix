# Local key import from Element (and why Element X cannot be)

Status: implemented for #1871 — Phase A (engine and IPC) and Phase B (React
surface, copy, browser tests). The surface is **Migrate to Koushi** in Account
Settings → Encryption, mounted again next to **Set up secure backup** on a first
sign-in.

The route order is a product decision, not a capability order, and it changed
after the engine landed: the key-file import is offered first and marked, the
recovery key second, this Element Desktop import third, and verifying from another
device last. Device verification is the route that keeps failing in the field
(#1780, #1279, and the nightly-qa `user_verification` failures #1323/#1534/#1169/
#1462/#1319/#1305), so the panel says it can stall and offers links back to the
first two routes on the same screen.

## Purpose and boundary

Moving from Element or Element X should not require the user to run
"export keys here, import them there" by hand. Koushi runs on the same machine as
the old client, so where the old client's decryption keys are readable on this
machine, Koushi takes them directly — and where they are not, it says so instead
of implying it looked.

The work splits along the same boundary as the rest of the desktop app:

| Layer | Owns |
| --- | --- |
| `apps/desktop/src-tauri/src/migration/` | Platform capability: locating another app's data, driving Element's own export over the Chrome DevTools Protocol, and staging what it found into a private file |
| `koushi-sdk` | SDK knowledge: producing the Element-compatible export envelope and importing it into the crypto store |
| `koushi-core` | Import semantics: `AccountCommand::ImportMigratedRoomKeys` reads the staged file, imports, deletes it, and settles the existing room-key import state machine (`RoomKeyImportRequested` → `RoomKeyImported` / `RoomKeyImportFailed`) |
| `apps/desktop/src-tauri/src/commands/migration.rs` | IPC: detection, staging, and the admission for the import command |

The adapter never calls SDK APIs and never decides what "imported" means; it
reports how many entries it staged and forwards the request.

The local sources are asymmetric, and the difference is deliberate:

| Source | Local import |
| --- | --- |
| Element Desktop (Electron) | Yes, through Element's own `exportRoomKeys()` (see below) |
| Element X (iOS) | No. Detected only, reported as unimportable |

Accounts with Element X data migrate by verifying from the old device and sharing
secrets in Koushi, not by parsing that app's files.

## Detection, platforms, and the import flag

Detection is read-only on every platform and reports counts and stable codes, so
the panel can say what is here without importing anything:

| Platform | Installed when | Running when |
| --- | --- | --- |
| macOS | `/Applications/Element.app` exists | `pgrep -f Element.app/Contents/MacOS/Element` matches |
| Windows | `%LOCALAPPDATA%\Programs\element-desktop\Element.exe` exists | `tasklist` lists `Element.exe` |
| Linux | one of `/usr/bin`, `/usr/local/bin`, `/opt/Element`, `/snap/bin` `element-desktop` exists | `pgrep -f element-desktop` matches |

The DevTools-protocol export itself runs on macOS only, so `import_supported` is
false elsewhere and the panel explains that instead of offering the route. The
import is additionally behind `KOUSHI_ENABLE_ELEMENT_DESKTOP_IMPORT=1`: the flag
is read at detection time and again before staging, so the route is hidden when it
is off and a stale renderer's request fails closed with
`migration_element_desktop_import_disabled`. Both flags travel as DTO fields; no
key material, path, or port leaves the adapter.

## Element X (macOS): detected, never imported

Element X is an iOS app, so on macOS its data sits in a group container:

```
~/Library/Group Containers/group.io.element/Library/Application Support/io.element.elementx/Sessions/<session-id>/matrix-sdk-crypto.sqlite3
```

Detection counts session directories holding a store and reports
`{sessions, unavailable_reason: "migration_element_x_encrypted_store"}`. Zero
means this account has no Element X data on the machine.

The store cannot be imported, and that is a property of the format rather than a
missing feature:

- `matrix-sdk-sqlite` encrypts every value it writes when a store passphrase is
  set. The stored bytes are MessagePack `{version, ciphertext, nonce}` — the
  measured leading bytes `83 A7 76 65 72 73 69 6f 6e` are `fixmap(3)` plus the key
  `"version"`, not a plaintext pickle map.
- Element X generates that passphrase at sign-in
  (`AuthenticationService.passphrase = encryptionKeyProvider.generateKey()`) and
  keeps it in its own keychain. Row keys (`session_id`, `room_id`) are keyed
  hashes too, so no column carries anything readable without it.

Reading the passphrase out of another app's keychain is out of scope: it needs its
own security decision, so the store is reported as present and unimportable
instead. Reporting it matters — a migration screen that silently found "0 keys"
would send the user looking for a bug instead of to the path that works.

macOS may also refuse to list the container at all (other applications' group
containers are protected). That is reported as
`unsupported_reason: migration_element_x_unreadable`, not as "no data".

## Element Desktop (CDP)

Element Desktop 1.12 is Electron; its keys live in Chromium IndexedDB
(`~/Library/Application Support/Element/IndexedDB/vector_vector_0.indexeddb.leveldb`),
which is not parsed directly. Instead the adapter asks Element's own web app:

1. Quit Element if it is running (`osascript -e 'tell application "Element" to quit'`,
   bounded wait; no force-kill).
2. Relaunch `/Applications/Element.app/Contents/MacOS/Element` with
   `--remote-debugging-port=<random 49152..=65535>` and the user's normal profile.
3. Poll `http://127.0.0.1:<port>/json/list` and accept only a `page` target whose
   URL is the Element webapp (`vector://vector/webapp` or
   `https://app.element.io/`). Any other target is ignored, and a
   `webSocketDebuggerUrl` that is not loopback on that exact port is refused —
   arbitrary pages are never evaluated.
4. `Runtime.evaluate` with `awaitPromise`, calling
   `mxMatrixClientPeg.get().getCrypto().exportRoomKeys()`, then
   `mxMatrixClientPeg.get().exportRoomKeys()` if the first fails. An empty result
   or an exception is a failure, never a partial success.
5. Quit Element, and relaunch it without the debugging port only if it was
   running before step 1.

While that instance runs, `<port>` is a local attack surface: any same-user
process can reach it. The export itself is bounded — the adapter waits at most
90 s for the webapp target and the DevTools round trip, and 10 s per quit
request — but the port closes only when that Element process exits. The adapter
never force-kills it: if Element ignores the quit request, the process keeps
running with the debugging port until the user quits or restarts Element, and the
second quit's failure is not escalated. This is the mechanism the issue
prescribes for a local tool, and it is why the port binds loopback only and the
target allowlist rejects anything that is not the Element webapp.

New dependencies for this path: `tokio-tungstenite` (WebSocket frames) and
`futures-util` (already in the workspace lock).

## Staging file and import

The adapter cannot use SDK types, so it hands Core a private file:

```json
{"version":1,"keys":[<Element ExportedRoomKey>]}
```

- Written with `tempfile`, which creates it `0600`; nothing else reads it.
- Core reads it once through `koushi_sdk::import_migrated_room_keys`, which
  decrypts nothing by hand: the staged keys are re-encrypted into a one-shot
  Element-format envelope with a random passphrase and imported through the same
  `import_room_keys` path a hand-picked Element export uses.
- Core deletes the staging file after the import, on success and on failure. The
  adapter deletes it when the command could not be submitted at all, and the app
  sweeps every leftover at startup plus files older than an hour on the next
  detection or staging call. Both matter: a command rejected before it reaches the
  handler (a session that ended between admission and dispatch) hands the file back
  to nobody, so startup is the only point that reliably runs afterwards.

Reported numbers, which Phase B must not conflate:

- `migration_import_element_desktop` returns `staged_keys`, `staged_rooms` —
  what Element exported.
- `RoomKeyImported { imported_count, total_count }` — what the crypto store
  actually took.

## Privacy

- Key material stays inside the adapter's staging file and the SDK import; it
  never reaches the webview, the diagnostics event stream, logs, or an error
  string.
- Detection returns counts, booleans, and stable codes
  (`migration_element_desktop_import_disabled`,
  `migration_element_desktop_platform_unsupported`,
  `migration_element_x_unreadable`, …) — no filesystem paths, no user id, no
  port, no key material.
- Tests build synthetic session directories and staging files in a temp
  directory, with synthetic Matrix ids under `example.invalid`. No automated test
  reads a real Element or Element X store.

## Limits

- The DevTools-protocol export runs on macOS only; Windows and Linux report
  `migration_element_desktop_platform_unsupported` on
  `element_desktop.unavailable_reason` and their installs are still detected.
- Windows checks `%LOCALAPPDATA%\Programs\element-desktop\Element.exe` and
  `C:\Program Files\Element\Element.exe`; Linux checks `/usr/bin`,
  `/usr/local/bin`, `/opt/Element`, and `/snap/bin`. Flatpak and AppImage
  installs are not found, so "no Element Desktop" means "not in these places".
- Element X stores are encrypted and stay unimportable; those accounts migrate by
  verifying from the old device, and the Element Web/Desktop export or the
  recovery key remains the fallback.
- Manual confirmation against a real, signed-in Element installation has not been
  performed as part of Phase A; the CDP conversation is covered by a fake
  loopback server only.
