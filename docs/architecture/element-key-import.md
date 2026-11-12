# Local key import from Element (and why Element X cannot be)

Status: implemented engine and IPC for #1871 (Phase A). No React surface yet, so
nothing here is a user-facing flow until Phase B wires the commands below.

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
process can reach it, and it stays open until the adapter quits Element (bounded
at 90 s). This is the mechanism the issue prescribes for a local tool, and it is
why the port binds loopback only and the target allowlist rejects anything that is
not the Element webapp.

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
- Detection returns counts and stable codes
  (`migration_element_x_encrypted_store`, `migration_element_x_unreadable`, …) —
  no filesystem paths, no user id, no key material.
- Tests build synthetic session directories and staging files in a temp
  directory, with synthetic Matrix ids under `example.invalid`. No automated test
  reads a real Element or Element X store.

## Limits

- macOS only. Other platforms report
  `unsupported_reason: migration_platform_unsupported`.
- Element X stores are encrypted and stay unimportable; those accounts migrate by
  verifying from the old device, and the Element Web/Desktop export or the
  recovery key remains the fallback.
- Manual confirmation against a real, signed-in Element installation has not been
  performed as part of Phase A; the CDP conversation is covered by a fake
  loopback server only.
