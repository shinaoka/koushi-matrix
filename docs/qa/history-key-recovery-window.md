# Historical-key recovery window against a local homeserver

Run the disposable Synapse scenario from the repository root:

```bash
node scripts/desktop-headless-local-qa.mjs --run --core \
  --server=synapse --scenario=history_key_recovery_window --timeout-ms=600000
```

The scenario is also registered in the nightly focused lane. It exercises
Koushi's production SDK client builder, encrypted persistent stores, and the
SDK Simplified Sliding Sync service; the first phase is SDK-level invitation-import evidence, followed by a
production Core-command recovery scenario. GUI wiring is verified separately. No classic sync or external homeserver is used.

Two disposable accounts bootstrap their cross-signing identities. The sender
creates two encrypted rooms with shared history and sends two historical
messages under separate Megolm sessions in each before inviting the recipient.
Actual encrypted bundles are uploaded and delivered through the homeserver.
The recipient's public media fetcher temporarily returns a non-NotFound error
so normal invite acceptance cannot finish importing. The oracle requires stored
bundle data, retained pending records, and undecryptable historical messages.
This proves delayed **import**, not delayed to-device receipt.

Only the existing pending records' acceptance timestamps are backdated through
the SDK's test-only crypto-store API: one to 29 days, the other to 30 days.
Room IDs, inviter identities, bundle contents and key-sharing flags are not
manufactured or altered. The original sync owner is stopped and stores closed;
the same device is restored from its encrypted persistent store. Normal startup
recovery must download the real 29-day bundle and decrypt both historical
messages, clear both pending records, and leave both 30-day messages
undecryptable. No backup recovery or key-request action is run on the recipient.
Exact boundary milliseconds and future timestamps remain covered by the
fixed-clock SDK unit tests.

The media gate counts only deferred attempts. After restoring, the default
downloader retrieves the encrypted bundle from the real server. Assertions
wait for authoritative store/decryption conditions within bounded deadlines;
they do not infer success from a fixed delay. Output is fixed tokens/counts
only, never SDK errors, identifiers, credentials, keys or message contents.

Required evidence tokens, enforced by the Node runner:

- `history_window_deferred=ok`: bundles exist but neither room's historical
  messages decrypt before restart.
- `history_window_29_days=ok`: the restarted same device downloads and imports
  the bundle and decrypts both historical messages at 29 days.
- `history_window_30_days_expired=ok`: startup removes the expired pending
  record and both messages remain undecryptable at 30 days.
- `history_window_cleanup=ok`: every created sync owner is stopped, account
  logout is attempted on failure and required on success, stores are closed,
  and temporary encrypted client stores are removed.
- `history_key_recovery_window=ok`: the entire scenario succeeds.

The outer runner always stops/removes the disposable Synapse container. The
client fixture always attempts cleanup even when a setup or oracle fails.
Persisted QA output passes the shared private-data checks before being written.

The scenario also runs production Core room commands against the same real
clients. Credential UI installation alone uses a narrow fixture hook; key
requests, authorization, Olm transport, responder export, import and state
projection are production code. Core owns each active Simplified Sliding Sync
service. A third bootstrapped regular member makes the small-room manual target
set differ from administrator-only startup. These accounts have owner-signed
devices but never manually verify each other; assertions before and after
recovery require their identities to remain unverified. The Node fixture prepares
30 joined members in a public room; the requester joins as 31. Lazy client membership
cannot substitute for the authoritative server member population.

The requester restarts twice from its encrypted same-device store while the
administrator is offline. The first restart sends only to the administrator,
retains the outstanding per-device nonce, and persists the automatic cooldown;
the second restart cannot increase that operation generation. When the
administrator's Core sync resumes, both distinct historical sessions decrypt.
An explicit request also recovers the original 30-day-expired invitation room.

The responder's encrypted rate ledger is seeded with 512 expired pairs, including
one for the requesting room and user. Real requests must recover keys for that
pair and for a new pair. With 512 unexpired pairs, an encrypted rate refusal
must reach the requester without completing its attempt or exporting a reply.
The ledger fixture is restored afterwards.

Negative cases change the real server's history visibility and membership,
and locally block an owner-signed requester device. Count-only Core
observations prove the affected room's incoming request was refused before the
oracle requires an empty recipient key store. A final pause occurs after a
real chunk's durable reservation but before SDK import: both clients restart,
only the responder cooldown clock fixture is advanced, and the exact persisted
cached response must import. Its unchanged response ID and durable Ack/deletion
tombstone are checked, and the exact encrypted payload row must be absent after
Ack; an empty stored row is insufficient. No expected response or imported key
is manufactured.

Additional required tokens:

- `history_recovery_offline_pending=ok`
- `history_recovery_startup_admin_retry=ok`
- `history_recovery_restart_cooldown=ok`
- `history_recovery_join_without_invite=ok`
- `history_recovery_without_manual_verification=ok`
- `history_recovery_31_members_admin=ok`
- `history_recovery_force_all=ok`
- `history_recovery_after_window_expiry=ok`
- `history_recovery_visibility_guard=ok`
- `history_recovery_membership_guard=ok`
- `history_recovery_blocked_device_guard=ok`
- `history_recovery_crash_replay=ok`

- `history_recovery_policy_race_guard=ok`: a requester leaving after the final
  identity refresh and before disclosure receives no historical keys.
- `history_recovery_identity_race_guard=ok`: rotating only the owner master
  identity while retaining device keys rejects a paused disclosure and a paused
  import. The newly queried identities remain cross-signed and unverified, so
  rejection proves exact pin checks rather than verification-violation handling.
