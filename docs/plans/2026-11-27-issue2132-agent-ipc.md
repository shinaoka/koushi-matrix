# Agent IPC: profiles, lease, push subscription (#2132) — design

Status: proposed, for independent review before any code. Stage 3 of epic #2124.
This plan never overrides the canon. The stage that implements it amends the
canon (Agent Path in `docs/architecture/overview.md`, "Wake-feed row and
subscription lifecycle" in `docs/architecture/state-machine.md`, Key Management
in `REPOSITORY_RULES.md`); this document touches none of them.

The Agent Path and wake-feed canon sections come from #2125 (branch
`docs/2125-agent-canon`, not yet on `next/agent-core` when this was written). The
design follows that text; rebase and re-check it when #2137/#2125 land.

## 0. Summary of decisions

| # | Decision |
| --- | --- |
| D1 | Protocol 2 is additive and is **negotiated per connection**. A `protocol: 1` hello is served exactly as today, answered `hello_ok{protocol:1}`, and its `status` reports `1`. Profiles, `subscribe`, `ack` exist only on a `protocol: 2` hello, and a v2 hello must carry a profile (`profile_required`). The launcher sends 2 only when `--profile` is given. |
| D2 | Every handshake error from a new host carries a `reason`. Version mismatch is `Unsupported/protocol_version` plus `supported: [..]`; a profile on a v1 hello is `Unsupported/profile_requires_protocol_2`; an invalid client name is `Unsupported/client_name`. The launcher validates the name before the handshake and tells the user what to do for each. |
| D3 | Profiles live in a new owner-written file `<data_dir>/mcp/profiles.json`, device scope, same fail-closed rules as `connections.json`, written only by the low-frequency owner path under a file lock. The binding to one account is immutable. Two profiles may bind the same account; they stay distinct because the grant subject is the profile id. |
| D4 | The profile secret is 256 random bits. The host stores only its SHA-256 verifier. The secret itself is written once to a `0600` file (`O_EXCL`) the plugin reads. Key Management's secret-store rule does not apply because it governs the at-rest unlock key, not authentication credentials; macOS and a headless host behave identically. |
| D5 | Every connection carries a `GrantSubject::{Client(name), Profile(id)}`. Every grant, approval, idempotency key, activity-page cursor and draft key is derived from the subject only; `client.name` becomes a display label that is not a key. Profile grants live in the profile row. `connections.json` is not touched. |
| D6 | Push is paced by the durable feed, not buffered. Core keeps at most `PUSH_WINDOW` (64, see Constants) unacknowledged rows in flight per subscription. `resync_required` is raised only for a cursor the feed can no longer serve, or a consumer that stalls. |
| D7 | Cursor acknowledgement: **explicit `ack`** (option 2). Core persists one acknowledged cursor per profile in its own file, never in `profiles.json`; a plugin acks at the strongest hand-off its harness can observe. |
| D8 | Lease: in-memory map, RAII guard owned by the subscription, released on any connection end. Requests are not leased. |
| D9 | Attachment fields (`message_kind`, `mime_type`, `size_bytes`) are recorded at intake and are owned by #2131, never read at push time. Absent fields mean "unknown". |
| D10 | UI: one new "Agent profiles" section on the existing AI assistants page, rendering a Rust `profiles` list. Create, copy snippet, connected, revoke. |
| D11 | Five PRs into `next/agent-core`: 1a wire and handshake, 1b profiles store and isolation (with an end-to-end create-and-use test), 2 subscription, 3 UI, 4 attachment end-to-end test. No subscription or UI before PR 2/3. |

### Constants

All tunables live in one module (`crates/koushi-mcp-host/src/profile_limits.rs`),
each with its reason next to it. They are not API and not settings. The other
sections refer to them by name.

| Constant | Value | Reason |
| --- | --- | --- |
| `PUSH_WINDOW` (`W`) | 64 rows | Starting value, not derived. Large enough to cover a burst of mentions in one sync, small enough that a stalled consumer pins little and redelivery after a resync is cheap. |
| `ACK_STALL` | 60 s | Starting value. Longer than a slow model turn and a harness restart, shorter than the time a missing wake-up would go unnoticed. |
| `WRITE_STALL` | 15 s | Starting value. A local socket write that blocks this long means the peer is not reading. |
| `WRITER_CHANNEL` | 128 frames | Twice `PUSH_WINDOW`, so a full window plus responses never blocks the reader loop. |
| `ACK_FLUSH` | 1 s | Bounds acknowledged-cursor writes; a crash redelivers at most the last second. |

## 1. The protocol

### Today

- `PROTOCOL_VERSION = 1` (`crates/koushi-mcp/src/wire.rs:10`). `Hello{protocol,
  token, client}` and `HelloOk{protocol, connection_id, app_version}` are in
  `wire.rs:142-175`. `ClientMessage::Hello` has no profile field.
- The host accepts only an exact match (`crates/koushi-mcp-host/src/server.rs:406`)
  and otherwise answers `Error{Unsupported, "...older than the running Koushi"}`
  after the token check (`server.rs:247-262`). The launcher accepts only an exact
  `HelloOk.protocol` (`crates/koushi-mcp/src/ipc.rs:119-127`).
- The connection loop is strictly one request at a time and watches the socket
  only for close while a request runs (`server.rs:288-340`). A push stream cannot
  fit in that loop (see section 4).

### Decision D1: additive protocol 2

- `PROTOCOL_VERSION` becomes the highest supported version, `2`, and the host gets
  `SUPPORTED_PROTOCOLS = [1, 2]`. The constant is **never** echoed on its own:
  the version is negotiated at `hello` and stored on the connection
  (`negotiated_protocol`). Both `hello_ok.protocol` and `status`'s `"protocol"`
  field (`server.rs:493` returns `PROTOCOL_VERSION` today) report the connection's
  negotiated value, so a v1 connection's `status` still says `1`. A test pins this
  for `status` as well as `hello_ok`.
- `ClientMessage::Hello` gains `profile: Option<ProfileHello>` with
  `#[serde(default, skip_serializing_if = "Option::is_none")]`. `ProfileHello` is
  `{ id: String, secret: String }`. `hello` otherwise keeps its fields.
- A hello with `protocol: 1` is handled by today's path unchanged. If it carries a
  `profile` it is refused with `Unsupported`, `reason: "profile_requires_protocol_2"`
  (a v1 peer cannot know the field), so a profile can never be presented on the
  legacy path and the refusal cannot be mistaken for a token rejection
  (`Unauthorized`).
- The host answers `hello_ok` with the protocol the client asked for. A v1 client
  therefore receives the same bytes as today. The v1 launcher compares
  `HelloOk.protocol == 1` and keeps working without a rebuild.
- A `protocol: 2` hello without `profile` is refused with `Unsupported`, `reason:
  "profile_required"`. There is no unprofiled v2 connection, which removes a path
  to test and a way to subscribe without a profile.
- **The launcher sends `protocol: 1` unless `--profile` is given**, and `2` only
  with it. A launcher newer than the host therefore keeps working without a
  profile, as the issue requires ("existing unprofiled MCP connections keep
  working"); only a profiled launch needs a v2 host.
- The existing per-launch token from `endpoint.json` is still required on both
  versions. The profile secret is additional proof, not a replacement: a process
  must have read the owner's endpoint file and know the profile secret.
- Message additions, all v2-only:
  - client → host requests `subscribe`, `unsubscribe`, `ack` (ordinary
    `Request{id, method, params}`; no new client message type);
  - host → client `Push{sub, rows, cursor}` and `SubEnd{sub, error}`.
  A v1 peer is never sent either, because it cannot have subscribed.
- `ServerMessage::Error` gains optional `reason` and `supported` fields
  (`skip_serializing_if`), so existing decoders still parse it. **Every handshake
  error from a new host carries a `reason`** (token and profile rejections are
  `Unauthorized` with `reason`; mismatches are `Unsupported` with `reason`).
  Errors without a `reason` can then only come from an old host.

### Decision D2: handshake errors, both directions

Reuse the closed `ErrorCode` set and the `reason` refinement the wire already uses
for `resync_required` (`wire.rs:79-85`), rather than add a code an old peer cannot
decode.

The host already answers `Error{Unsupported, CLIENT_NAME_RULE}` for an invalid
client name (`read_hello`). That is also `Unsupported`, so without a `reason` it
would be misread as a protocol mismatch. Hence:

- The launcher validates the client name against the same rule **before** it opens
  the handshake, so a bad name is reported locally and never reaches the host.
- A new host answers an invalid name `Unsupported/client_name`.

| Situation | Wire | Launcher tells the user |
| --- | --- | --- |
| Profiled launcher (v2) in front of a host that supports only `[1]` | The old host answers its legacy `Unsupported` with no `reason`, after the token check. A reply without `reason` can only come from an old host (all new-host handshake errors carry one), so the client maps it to `Unsupported/protocol_version`. | "This Koushi is too old for agent profiles. Update Koushi." |
| Unprofiled launcher (v1) in front of any host | Served as today. | n/a |
| Number outside `[1,2]` (0, 3) | `Unsupported/protocol_version`, `supported:[1,2]`, after the token check; connection closed. | "Koushi and the launcher do not share a protocol. Register the launcher again, or update Koushi." |
| v1 hello carrying `profile` | `Unsupported/profile_requires_protocol_2` | Internal; a correct launcher cannot produce it. |
| v2 hello without `profile` | `Unsupported/profile_required` | Internal; same. |
| New launcher, old host: host replies a protocol the client did not ask for | `Unsupported/protocol_version` on the client side. | "Update Koushi." |
| Launcher older than the host (host no longer serves its version) | `Unsupported/protocol_version`, `supported:[...]`. | "Register the launcher again" (the old launcher prints the host's message text). |
| Invalid client name | `Unsupported/client_name` (launcher normally catches it first). | "The assistant name is not allowed: <rule>." |
| Bad token | `Unauthorized/token` | existing behaviour |
| Bad profile id or secret | `Unauthorized/profile_secret` (identical for both) | "The profile secret was rejected. Create the profile again." |

- The existing "outdated launcher" note on the settings page
  (`note_outdated_launcher`, `server.rs:247`) is kept for the protocol-mismatch case.
- The check stays after the token, so an unauthenticated peer cannot probe the
  supported set and cannot put a name on the settings page.

### Decision: unprofiled stays byte-for-byte

Test evidence, not argument: PR 1a records golden transcripts of a v1 session
(hello, `tools/call list_accounts`, `status`, an error) from `HEAD` before the
change and asserts the new host reproduces them (section 10).

"Byte for byte" excludes two fields that are not stable between runs:
`connection_id` (it contains a pid) and `app_version`. The transcript comparator
masks exactly those two (`hello_ok.connection_id`, `hello_ok.app_version`) and
nothing else. Every other byte, including `status.protocol == 1`, is compared.

## 2. Profiles

### Where they live (D3)

- File: `<data_dir>/mcp/profiles.json` (`0600`, parent `0700`), next to
  `connections.json` (`crates/koushi-mcp-host/src/grants.rs:1-14`). Not in
  `SettingsValues`: settings must not contain Matrix user ids, and the bound
  account id (`account:<user id>`) embeds one.
- Scope: this device. Profiles are not synced and not part of any account's
  encrypted store. A profile outlives sign-out of its account and is dead until
  that account is signed in again.
- Writers: only the low-frequency owner path (create, revoke, grant edit), and
  each write is a read-modify-write under an exclusive file lock
  (`profiles.json.lock`, `flock`) that re-reads the file inside the lock. Nothing
  on the request path or the ack path writes this file, which keeps "no IPC method
  changes the store" literally true and means a stale writer in another process
  (the Linux CLI of #2109) cannot resurrect a profile the owner revoked: the
  revoke is a delete under the lock, and any later writer re-reads first.
- Rules copied from the grant store: schema number, unknown keys preserved, a
  missing/malformed/newer file grants nothing, the app never overwrites a file it
  could not parse, every request re-reads (or checks a generation-stamped
  cache of) the file so a revocation applies to the next request.
- Row: `profile_id` (`prof_` + 128 random bits, hex), `label` (owner text, sanitised
  with `consent::sanitize`, shown only in the owner's UI), `account_id`, the five
  grant flags, `secret_verifier`, `created_at_ms`. The secret is not in the row.
  The acknowledged cursor and the creation baseline (`feed_generation`, head
  cursor) are **not** in this file: they live in the cursor file of section 6.
- Two profiles may bind the same account (for example a reader and a sender
  assistant, or two harnesses on one agent account). That is allowed because the
  grant subject is the profile id, not the account: each has its own grants,
  approvals, idempotency keys, cursors, lease and cursor file. Neither sees the
  other, and sending from the same account is the account's, not the profile's,
  concern (messages are attributed to the account).

### Binding to exactly one account (D3)

- `account_id` is set at creation and **immutable**. Re-binding is revoke plus
  create. This removes the question of what happens to grants when a binding moves
  and makes "(profile, account)" a key with one possible account.
- Core refuses to create a profile for an account that is not currently an agent
  account (`AgentAccounts::role_for`, `crates/koushi-state/src/state/settings.rs:1045-1065`),
  so the owner's own account cannot be bound by mistake.
- The agent-account check is repeated on every request. If the owner removes the
  role later the profile answers `not_permitted` with reason `not_agent_account`
  until the role is restored. A signed-out or locked account is `account_unavailable`,
  as today.

### The secret (D4)

- Generated in the host from the OS CSPRNG: 32 bytes, base64url. Stored as
  `SHA-256(secret)` in the row. A 256-bit uniformly random secret needs no slow
  KDF. The comparison is constant time over the **hashed** values (hash the
  presented secret, compare 32 bytes with `constant_time_eq`, `server.rs:441-452`),
  so length and content do not leak.
- Unknown id: the host verifies against a fixed dummy verifier and takes the same
  path, so timing does not disclose which ids exist.
- Failed authentication: no lockout. A 256-bit secret does not need one, and a
  lockout would let anyone holding the endpoint token lock the real plugin out.
  Instead each rejection is logged (id prefix only, never the presented secret)
  and the row records `last_rejected_ms` in the cursor file (not
  `profiles.json`, section 6) and surfaces it in the UI as a "last rejected"
  time, so the owner sees an attacker or a stale launcher.
- Creation order and file hygiene: the secret file is created with `O_CREAT|O_EXCL`
  and mode `0600` at open time (not chmod afterwards; the directory is `0700`),
  written, `fsync`ed, then the verifier row is written under the lock. If the
  process dies in between, an orphan secret file exists with no row, which
  authenticates nothing and is removed on the next start. Revoke runs in the
  reverse order: the row is deleted first (so authentication stops), then the
  secret file is unlinked. The launcher refuses a secret file whose owner is not
  the current user or whose mode has any group/other bit, and does not follow
  symlinks.
- Delivery: at creation the host writes the secret once to
  `<data_dir>/mcp/profiles/<profile_id>.secret` (`0600`, directory `0700`) and the
  launch snippet carries only the **path**. The secret never goes in argv, the
  clipboard, an environment variable, a log, or a snapshot DTO; it is not shown in
  the UI. This extends the rule the repository already holds for passwords and
  recovery keys.
- Why not Keychain, and what Key Management says: the secret **does** authorise
  returning decrypted message data and sending as the account, so it is not
  harmless. Key Management still does not apply, for a different reason: that rule
  governs the *at-rest unlock key* for the SDK and search stores, not authentication
  credentials to a running host. And an attacker with the same local user already
  holds the `endpoint.json` token, so a Keychain item would not raise the bar
  against that attacker. The host needs a `0600` socket in a `0700`
  directory and a per-launch token. The plugin is a separate non-interactive
  process; a Keychain item would need an ACL grant or a prompt per harness and
  would not exist at all on a headless host. A file the plugin reads is also what
  the launcher already does with `endpoint.json`. Result: macOS and a headless
  host share one mechanism, and **this stage does not depend on, or decide,** the
  open headless store-key question (0600 key file versus systemd credentials). It
  must say so in the canon so the two are not conflated.
- Verifier-only storage means a leaked `profiles.json` cannot authenticate. A leaked
  secret file can, within the limits in section 3.
- Rotation is revoke plus create; there is no rotate-in-place in Release 1.
- Wrong secret and unknown profile id produce the identical
  `Unauthorized{reason:"profile_secret"}` so ids cannot be enumerated.
- **Same-user isolation does not exist.** The agent process runs as the owner's OS
  user and can read every `profiles/*.secret`, `endpoint.json` and the socket.
  "A profile cannot reach another profile" therefore holds for an honest plugin
  that only opens its own file, not for a malicious or prompt-injected agent that
  goes looking. The UI text must say "limits what an assistant is allowed to do",
  not "prevents an assistant from reaching other profiles".

### The grant subject (D5, resolves the isolation blocker)

Today `client.name` is the key for grants and several other things. Seven places
besides the grant lookup read it: `list_accounts` (`store.account(&client.name,
…)`, `handlers.rs:112`), `read_recent_activity` (`store().allows(&client.name,
…)`, `:1647`) and its `activity_pages.read(&client.name, …)` (`:1683`),
`DirectRetryKey.client` (`:809`, `:943`), `AssistantDraftRequest.client_name`
(`:1147`), and `consent.rs`'s `pending`, `denied`, `seen` and `remove_client`.
`required_account_id` takes no client and decides no grant, so it cannot be the
choke point. A profiled connection that claimed the name "Claude Code" would pass
the client-name grants of another connection; a differently named one would fail
`account_unavailable`. The design is therefore changed at the identity:

- `ClientIdentity` (`consent.rs:85`) is split into `label: String` (display only,
  the sanitised client name) and `subject: GrantSubject`, where
  `enum GrantSubject { Client(ClientName), Profile(ProfileId) }`. A profiled
  connection has `Profile(id)`, an unprofiled one `Client(name)`.
- `label` is a private field with a `Display`-only accessor. It does not implement
  `Hash`, `Eq` or `Into<String>`, and no function that takes a key accepts it. The
  compiler therefore rejects any attempt to key a grant, approval, idempotency
  key, cursor or draft on the label. `ClientName` and `ProfileId` are distinct
  newtypes, so a profile id can never be passed where a name is expected either.
- Every site above takes a `&GrantSubject`:

| Site | Keyed on |
| --- | --- |
| Grant lookup and `authorize` (`consent.rs`) | `(subject, account, need)`; `Client` reads `connections.json`, `Profile` reads the profile row |
| `ConsentBroker` `pending`, `denied`, `seen`, `remove_client` (renamed `remove_subject`) | `subject` |
| `list_accounts` | `Client`: today's listing. `Profile`: exactly one entry, the bound account, with the profile's permissions. It never sees another account id. |
| `read_recent_activity` permission check and `activity_pages` cursor | `(subject, account)`. A cursor minted for one subject is rejected for another, so a name collision cannot replay a page. |
| `DirectRetryKey` (send idempotency) | `(subject, account, key)` |
| `AssistantDraftRequest` / `get_draft` | `(subject, account)`; the draft row stores the subject |
| Settings "assistants" list | `Client` rows as today; `Profile` rows from the profile store |

- Profile connections resolve `account_id` through the same subject: the bound
  account is the only one any profile site can resolve. `required_account_id` stays
  as the *argument* check (below); it is no longer claimed to decide grants.
- `account_id` stays **required** in every tool schema. A profile does not make it
  optional or default it. A value other than the bound account is
  `not_permitted/profile_account`, never "omitted means the bound account". The
  check runs before authorisation, so the error shape does not reveal whether
  another account exists.
- **No IPC method creates, edits or widens a profile.** There is no `profile/*`
  method. The only writers of `profiles.json` are owner paths: the Tauri commands
  in section 8 (macOS) and the CLI (#2109, Linux). A grant widening by the approval
  dialog is also owner-answered (below).

## 3. Grants keyed by (profile, account)

- Coexistence, not migration (D5). `connections.json` schema 3 stays as is and
  keeps serving unprofiled connections by client name. Profile grants are the flat
  flags in the profile row. Nothing is converted, because a client-name grant
  proves nothing about which session holds it; copying it into a profile would
  carry the leak this stage exists to remove. The owner re-grants per profile.
- Unprofiled connections: unchanged. They authorise by client name, can still
  reach every account their grant covers, including an agent account, and remain
  exposed to the shared-name leak described in the issue. This stage does not close
  it for them; it gives agents a path that is not exposed. The settings page says
  which assistants are profiled.
- A profiled connection's name is a label only (the type split above). It is never a
  key anywhere, so a profile can neither inherit nor leak a "Claude Code" grant, and
  a client-name connection can never use a profile grant: the `Client` subject has no
  path to the profile store.
- Initial grants are chosen by the owner when creating the profile (checkboxes;
  `read` on, others off by default). A missing permission later goes through the
  existing `ConsentBroker` approval dialog with the key widened from
  `(client, account, need)` to `(subject, account, need)`
  (`consent.rs:198-203`, `385-` `authorize`). The dialog names the profile label and
  the bound account. In `FailClosed` mode (a host without a consent surface) it
  answers `consent_required`, as today.
- Revocation. The owner's revoke command deletes the row and the secret file under
  the store's write path, then signals every live connection of that profile
  (a per-profile cancellation token in the host). Those connections close, a
  subscription ends with `SubEnd{error: unauthorized/profile_revoked}`, and the
  lease is released. Independently, every request and every push batch re-checks
  the row, so a revoke is effective even if the signal is missed. Lowering a single
  flag (for example `read`) ends a live subscription at its next batch.
- In-flight requests on revoke: a `send_message` that has already started sending
  **cannot be recalled** and may complete (the revoke stops the next request, not
  the one on the wire). Long polls (`wait_for_events`, `wait_for_mentions`) and
  pending approval dialogs stop at their existing post-wait re-check of the row and
  return `unauthorized`/`profile_revoked` without returning data. The owner-facing
  text for revoke says this.
- **An attacker with the profile secret** (and, necessarily, the per-launch token
  and a connection to the `0600` socket as the same OS user) can: act on the bound
  account with the profile's granted flags only; hold the lease and so deny the real
  plugin its subscription (`profile_busy`; the owner sees "connected" and revokes);
  acknowledge cursors, which can suppress wake-ups for that profile but reads
  nothing; open request-only connections.
  **Cannot:** reach any other account or learn it exists; use any permission not in
  the row; create, widen, rebind or delete any profile or grant (no IPC method);
  read or recover the secret (only the verifier is stored); affect another
  profile's lease or cursor; subscribe twice.
  **Out of scope, same as today's token:** a malicious process of the same OS user,
  including the agent itself, can read every profile's `0600` secret file under
  `profiles/` and `endpoint.json`, and so can act as any profile. The boundary is
  accident and least privilege (an honest plugin only opens its own file), not
  same-user isolation. "Cannot reach another account" above holds for the holder of
  *one* secret. The settings text must not claim more.

## 4. The push subscription

### Connection restructuring

The strict one-request loop (`server.rs:288-340`) cannot interleave pushes. The
connection becomes: one writer task owning the write half and fed by a bounded
channel (capacity `WRITER_CHANNEL`); the reader loop dispatching requests; the
subscription pump, when present, sending into the same channel. Responses stay
serialised with the existing close/shutdown watches. A v1 connection uses the
same code with no pump, so its observable behaviour is unchanged (proved by the
golden transcripts).

### Shape

- Request `subscribe {after?: string}`. `after` omitted resumes after the
  profile's acknowledged cursor; `"retained"` starts at the oldest retained row;
  an `mf1` cursor starts after that row (replay). Validation is the feed's own
  (`CursorRejection`, `crates/koushi-core/src/mention_feed.rs:149-178`).
- Response `{sub, start_cursor, window}`. At most one subscription per connection
  (`sub` is a counter for forward compatibility).
- Stream: `Push{sub, rows:[Row..], cursor}` in per-account sequence order. A row is
  the existing wake-feed row (`handlers.rs:1766-1778`) plus the attachment fields
  of section 7. A row's `cursor` is `feed.cursor_for(seq)`. Rows carry no message
  body; the agent reads it with the existing tools under its `read` grant.
- `unsubscribe {sub}` ends the stream and releases the lease; the connection stays.
- `SubEnd{sub, error}` ends a stream for a typed reason: `resync_required` (below),
  `unauthorized` (revoked), `not_permitted`, `account_unavailable` (signed out,
  locked, role removed, feed unhealthy). The lease is released with it.

### Cursor and window

- The cursor is the wake feed's `mf1` cursor, bound to the account and the log's
  generation (`mention_feed.rs:47`, `281`). No new cursor space.
- Window: at most `PUSH_WINDOW` (64) rows sent and not yet acknowledged. When the window is
  full the pump stops reading the feed. Nothing is buffered in memory beyond that
  window and the 128-frame channel, because the feed is already a durable log
  (1000 rows or 30 days, `mention_feed.rs:40-43`). Backpressure replaces
  buffering, so a slow consumer cannot lose a row by an overflow.
- The pump reads with the feed's existing wait (`MentionFeed::wait`,
  `mention_feed.rs:413`), re-checks health, account readiness and the profile's
  `read` flag before every batch, as `wait_for_mentions` does after its wait
  (`handlers.rs:1780-1804`).

### Falling behind: `resync_required`

A subscription ends with `SubEnd{resync_required, reason}` only when continuing
would skip or stall:

| reason | Trigger | Consumer action |
| --- | --- | --- |
| `expired_cursor` | The unacknowledged position fell out of retention (1000 rows / 30 days) before the consumer acked. Rows were lost to the feed. | Resubscribe with `after:"retained"`, treat history as possibly missed, de-duplicate by event id. |
| `generation_changed` | The live feed's log is not the one the stream started on (re-checked before every page and while idle). | Same. |
| `ack_stalled` | The window stayed full with no `ack` for `ACK_STALL` (60 s). | Resubscribe with `after` omitted; unacked rows are redelivered. |
| `write_stalled` | A frame could not be written for `WRITE_STALL` (15 s). Best-effort at the peer: the `SubEnd` is queued through a reserved slot but waits behind the unread frames; if the peer does not resume reading, the connection closing is the signal. | Same. |

For the last two, nothing is lost: Core still holds the acknowledged cursor, so
resuming redelivers everything after it. Rows are never silently skipped: they
are either delivered, redelivered, or the loss is named (`expired_cursor`,
`generation_changed`) in a typed end. The reason tokens and message format follow
the existing `resync_required` convention (`wire.rs:79-85`).

### What delivery does and does not guarantee

- **Does:** in order, per account; at-least-once from Core to the plugin up to the
  acknowledged cursor; no row skipped while retained; durable across a Koushi
  restart; one stream per profile.
- **Does not:** exactly-once (a row can be redelivered after a crash, a resync, or
  a lost `ack`; de-duplicate by event id, and by cursor for `invite` rows, which
  have no event id); delivery into the agent (Core ends at the plugin; what the
  harness does next is section 6); delivery of anything outside the wake feed or
  after retention (30 days / 1000 rows); ordering across accounts (a profile has
  one).

## 5. The lease

- State: an in-memory `HashMap<ProfileId, ConnectionId>` in the host's `Shared`
  (`server.rs:150`), matching the state-machine sketch `free → held → free`. Not
  persisted: a restarted Koushi has no holders, which is exactly the "no stale lock"
  requirement.
- Acquire at a successful `subscribe`. Second `subscribe` on another connection of
  the same profile fails with the new `ErrorCode::ProfileBusy` (`profile_busy`).
  That connection keeps working for requests. A profile bound to another account
  is another key, so leases never interact.
- The holder owns an RAII `LeaseGuard`. It is dropped when the connection task ends
  by any route: clean close, EOF, read error, shutdown signal, task abort
  (`accept_loop` aborts tasks at shutdown, `server.rs:196-231`). A crashed plugin
  closes its socket and the guard drops. A plugin that hangs without closing is
  covered by `ack_stalled` / `write_stalled`, which end the subscription and so
  release the lease.
- Revocation and role removal release it through the same `SubEnd`. A revoke queues `SubEnd{unauthorized/profile_revoked}` before the connection closes, however it is noticed (a reserved writer slot makes this independent of queue state).
- Requests are deliberately not leased: a profile's plugin may open a second
  connection for tool calls (the MCP launcher is a different process from the
  channel adapter), and all of those are bound to the same account anyway. The
  lease only serialises the one resource that must not be double-consumed.
- `profile_busy` carries no holder detail.

## 6. Cursor acknowledgement

### Facts (from #2126-#2128)

| Harness | What a plugin can observe | Source |
| --- | --- | --- |
| Claude Code (Channels) | **No delivery receipt.** The channel notification can be dropped silently. | #2126 |
| Pi | `sendUserMessage()` injects a real user message; the plugin knows it happened. | #2127 |
| Codex | App server reports `turn/started`. | #2128 |
| DeepSeek Harness | **Not verified.** | #2129 |

### Options, as the issue lists them

1. **Acknowledge when the plugin has handed the event to the harness.**
   Core cannot observe a hand-off, so this means Core advances when it *writes* the
   frame, or the plugin has no say. For Claude Code that is at-most-once at the
   wrong boundary, with the loss invisible to everyone. Rejected.
2. **Acknowledge on an explicit `ack`.**
3. **Acknowledge when the agent replies.** Core could infer it from the agent's own
   send in the room. Rejected: not every wake deserves a reply (an `invite` row is
   information only, an agent may decide to stay silent, and one reply can answer
   several rows), so a reply-based rule redelivers forever or acknowledges the
   wrong rows. It also lets a model's behaviour move a cursor.

### Recommendation D7: option 2, explicit `ack`

- `ack {cursor}` is **cumulative**: it acknowledges every row up to and including
  that row, which is sound because sequence numbers are monotonic per account. A
  cursor older than the current acknowledged one is a no-op (idempotent). A cursor
  newer than anything sent on this subscription is `Unsupported`, reason
  `ack_ahead`.
- Core persists one acknowledged cursor per profile in **its own file**,
  `<data_dir>/mcp/profile-state/<profile_id>.json` (`0600`), which also holds the
  creation baseline (`feed_generation`, head cursor) and `last_rejected_ms`. It is
  not in `profiles.json`: that file is owner-written and low-frequency, and an ack
  every second would both break "no IPC method changes the store" and open a
  read-modify-write that could resurrect a profile revoked from another process.
  Each cursor file is written only by the host that owns that profile's lease, with
  an atomic temp-and-rename, coalesced (at most one write per second, flushed when
  the subscription ends). A crash can lose the last second of acks, which causes
  redelivery, never loss. The file is deleted at revoke; a state file without a
  profile row is ignored and swept at start, so a late write cannot make a revoked
  profile reappear (authentication never reads this file).
- `subscribe` without `after` resumes after the acknowledged cursor. A new profile's
  baseline is the feed head at creation, stored in the same file, so rows that
  arrive between creation and the plugin's first start are delivered, and old
  history is not replayed.
- The meaning of `ack` in the canon: *"the plugin asserts that the harness has
  accepted this row, by the strongest signal that harness gives it."* Core
  guarantees at-least-once **to** that assertion and nothing past it. The canon text
  must say this and must not list per-harness ack points as normative.
- Under option 2, per harness:
  - **Pi:** ack after `sendUserMessage()` returns. The injected user message is a
    real turn input, so redelivery after a crash is the only duplicate.
  - **Codex:** ack when `turn/started` is observed for the turn that carries the
    row. An event the app server rejects is not acked and is redelivered.
  - **Claude Code:** there is no receipt to wait for, so the adapter acks when the
    channel notification has been written. This is **at-most-once at the harness
    boundary**: Core cannot detect a notification Claude Code drops. The canon must
    state that Claude Code's guarantee ends at the plugin. The safety net is that
    nothing is deleted on ack: `wait_for_mentions` with a cursor still returns the
    retained rows (`handlers.rs:1731`), and the adapter's agent-facing text can tell
    the agent to reconcile with it. Whether to do that is #2126's decision.
  - **DeepSeek Harness (#2129, unverified):** Core's rule does not change, because
    it is harness-neutral: the adapter acks at whatever its `agent.send()` result
    gives it. If #2129 finds a receipt it acks on the receipt; if not, it behaves
    like Claude Code. #2129 is Release 2, so it is not on this stage's path. The
    only thing that would force a redesign is a harness that cannot say even that a
    call returned; none of the four is described that way.
- **Record in the canon:** the choice, the cumulative semantics, the persisted
  per-profile cursor, and the Claude Code limitation. This replaces the
  "Cursor acknowledgement (open decision)" block and the "acknowledged"
  state's "target semantics: open" text in the state machine, and Open decision 4 in
  Agent Path. That amendment ships in PR 2.

## 7. Attachments in a pushed row

- Fields: `message_kind` (closed set: `text`, `notice`, `emote`, `image`, `file`,
  `audio`, `video`, `location`, `other`), and for media `mime_type` and
  `size_bytes`. All optional on the wire (absent means unknown). No filename, no
  body, no caption, no bytes.
- **Source (D9): recorded at intake, and owned by #2131, not by this stage.** The sync handler that calls
  `MentionFeed::record` already holds the decoded event and its `info`
  (`crates/koushi-core/src/mention_feed/intake.rs`, `MentionCandidate` at
  `mention_feed.rs:84`). The fields are added to `MentionCandidate` and
  `MentionEntry` as `#[serde(default)]` so existing logs still load. Reading at push
  time was rejected: it adds an async fetch, a decrypt and possible network to the
  push path; it fails for exactly the rows redelivered after a restart or a resync;
  and the event can be redacted or gone from the cache by then. Intake time has the
  content for free and the answer is stable across redelivery.
- Consequence: the intake fields move into #2131 (they change `MentionCandidate`
  and the persisted entry, which is that issue's surface). This stage does not edit
  intake. If #2131 lands without them, the fields are absent and rows push with
  "unknown" until it does; PR 4 is then only the end-to-end test. Reading at push
  time stays rejected.
- Encryption at rest: the feed log is committed to the account's encrypted store
  (`mention_feed.rs:7`), so the new fields inherit that. They are metadata only
  (kind, MIME type, size); no filename, body or bytes are added.
- A row whose fields were never recorded (written before #2131, or an `invite`) still
  pushes with them absent. A row whose event is no longer available needs no special
  case: the push never touches the event.
- `mime_type` and `size_bytes` are the sender's declared `info` and are a hint,
  not a guarantee. `get_attachment` returns the real values for the fetched file
  (`handlers.rs:1873-1880`). Plugins and agents must not trust the pushed values for
  anything but triage.
- Fetching: `get_attachment` (`handlers.rs:1831`) under the profile's `attachments`
  grant. Its authorisation call moves from the client-name path to the same
  profile-aware `authorize`. The path it returns is inside the media cache
  containment check (`handlers.rs:1909`), unchanged.
- The harness `content` is built by the adapter (#2126) from the row. That row has
  no body, so the adapter's text tells the agent which room and event to read; this
  stage fixes only what the row carries.

## 8. Settings UI (Release 1, macOS)

Placement: a new "Agent profiles" section on the AI assistants page
(`apps/desktop/src/components/user-settings/AiAssistantsSection.tsx`, which already
hosts `AgentAccountsSection` and `AllowedSendersSection`), implemented as its own
component `AgentProfilesSection.tsx` beside them. No new page.

Rust facts the UI renders (the epic's rule: Core decides, React draws). They
extend `McpAssistantsState` (`consent.rs:148-160`) with `profiles:
Vec<ProfileView>`; the Tauri DTO and `domain/types.ts` mirrors follow
`docs/agents/state-ownership.md`:

- `profile_id`, `label`, `account_id` (the UI already resolves the account name from
  its account tabs), the five grant flags;
- `status`: `ready`, `account_unavailable`, `not_agent_account` (from Core's checks,
  section 2);
- `connected`: at least one live profiled connection; `subscribed`: the lease is
  held; `last_seen_ms`;
- `snippets`: the launch snippets per client, built in Rust like
  `registration.rs` (`crates/koushi-mcp-host/src/registration.rs:75-120`), with the
  launcher path and the secret file path substituted. React copies the text
  verbatim and computes none of it.
- The set of accounts that may be chosen: only agent accounts. A Rust list, so React
  does not re-derive the role check.

Owner actions, as Tauri commands forwarded to the host, which owns the store (like
the existing approval answers):

1. **Create:** label, bound agent account, initial grants. Returns the profile id and
   the secret file path (never the secret).
2. **Copy launch snippet:** per client; for Claude Code the snippet is the launcher
   plus `--profile <id> --profile-secret-file <path>`.
3. **Status:** `connected` / `subscribed`, read from the host's live state through
   the existing forwarder (`apps/desktop/src-tauri/src/mcp/mod.rs:163`).
4. **Revoke:** confirmation dialog, then the revoke path of section 3.

All text goes through the message catalog with an English baseline; snippets are not
product prose and stay identical in every locale, as `registration.rs` states. The
launcher gains `--profile` and `--profile-secret-file`; it reads the file itself and
sends `hello{protocol:2, profile}`.

Linux: the same host functions behind a CLI are #2109. This stage ships the host API
and no CLI.

## 9. Staged plan inside the stage

All into `next/agent-core`. Each leaves Koushi working and ships unreachable until
PR 3 gives the owner a way to create a profile.

**PR 1a — Wire and handshake.** `wire.rs` (v2 hello with `profile`, `Error`
`reason`/`supported`, `ProfileBusy` reserved), per-connection `negotiated_protocol`
in `hello_ok` and `status`, `server.rs` handshake (`SUPPORTED_PROTOCOLS`, the error
table of D2), the launcher's negotiation (1 unless `--profile`, client-name
validation before the handshake, user messages). *Proves:* golden v1 transcripts
(with `connection_id` and `app_version` masked), `status.protocol == 1` on v1,
typed mismatch both ways. *Out:* the profile store, isolation.

**PR 1b — Profiles store and isolation.** `profiles.rs` (store under file lock,
verifier, secret file with `O_EXCL`, creation and revoke as Rust functions only,
failed-auth handling), `GrantSubject` and the rewrite of every site in the table of
section 2, profile grants through `ConsentBroker`, `list_accounts` and the argument
check, the cursor-file module (baseline only; ack is PR 2). *Proves:* the
two-directional subject tests, the per-tool table, revocation, and **an integration
test that creates a profile through the Rust API and then uses it end to end**
(launcher hello with `--profile`, `list_accounts`, a read), so the profile path is
exercised before any UI exists. *Out:* `subscribe`, `ack`, lease, UI, canon edits
beyond the profile paragraph.

**PR 2 — Subscription, lease, ack.** Connection restructuring (writer task), `subscribe`,
`unsubscribe`, `ack`, `Push`, `SubEnd`, `LeaseGuard`, per-profile acknowledged cursor
(its own file), window and stall handling, Core's `MentionFeed` read-after primitive
if `wait` is not enough. The canon amendment for acknowledgement and the lease ships
here. *Proves:* the lease, resume after ack, slow consumer. *Out:* UI, attachments.

**PR 3 — Settings UI.** `profiles` in `McpAssistantsState`, Tauri commands, DTO mirrors,
`AgentProfilesSection`, snippets, catalog strings. The create and revoke commands are
added to the Tauri **capability allowlist** for the settings window only, with a test
that remote content (an `http(s)` origin or an iframe/webview of agent output) cannot
invoke them: the capability file is asserted to scope them to the local window, and a
test calls them with a remote-origin context and expects refusal. *Proves:* the
headless UI test. *Out:* Linux CLI (#2109).

**PR 4 — Attachment end-to-end test.** Depends on #2131, which owns the intake fields
and the row type. Adds the pushed-row assertion and the `get_attachment` end-to-end
test. If #2131 has not landed the fields, this PR waits for it; it does not add them.

Why this order: PR 1b contains every authorisation change, so the independent review
can concentrate there (1a is wire only); PR 2 adds concurrency without adding authority; PR 3 and PR 4
add no new trust.

## 10. Test strategy

Levels: **Core unit** (`koushi-core`, `koushi-mcp`), **host/bridge integration**
(`crates/koushi-mcp-host/tests/bridge`, using `tests/support` and its intake/registry
fakes, on the disposable local homeserver where a real event is needed), **headless
UI** (vitest in `apps/desktop`, beside `AiAssistantsSection.test.tsx`).

| Acceptance bullet | Level | Test and fixtures |
| --- | --- | --- |
| Two profiles bound to different accounts cannot reach each other's account or the owner's | Host integration (PR 1b) | Three accounts (owner, agent A, agent B) from `tests/support`. The per-tool table below; `list_accounts` returns one entry; a profile whose secret is wrong or id unknown gets the identical `unauthorized/profile_secret`. |
| A second subscriber gets `profile_busy` | Host integration (PR 2) | Two connections with the same profile; the second `subscribe` is `profile_busy` and the same connection can still `read_room_messages`. A third connection for profile B subscribes successfully. |
| Closing the first connection releases the lease | Host integration (PR 2) | Drop connection 1 (also abort its task, and a variant that closes the transport without a clean shutdown); the second `subscribe` now succeeds. Unit: `LeaseGuard` drops on task abort. |
| Reconnecting resumes after the acknowledged cursor | Host integration + Core unit (PR 2) | Feed fixture with rows 1-5; subscribe, receive 1-5, ack 3, close; resubscribe delivers 4-5. Restart the bridge with the persisted cursor file and repeat. Unit: coalesced cursor-file write and flush at subscription end; `ack_ahead` and stale-ack no-op. |
| A slow consumer gets `resync_required` rather than losing rows | Host integration (PR 2) | A fake transport (tokio duplex with a tiny buffer and a reader that stops) and a paused clock. Window: with W+10 rows and no acks the pump stops at W and no row beyond it is lost; after 60 s `SubEnd{resync_required, ack_stalled}`; resubscribe redelivers from the acked cursor with no gap. A feed fixture whose retention evicts the acked position yields `expired_cursor` with no silent restart at head. |
| An unprofiled MCP connection behaves as before | Host integration (PR 1a, kept in CI) | Golden transcripts of a v1 session recorded from the commit before PR 1a (hello, `list_accounts`, a read, `status`, a malformed request, a wrong token). Assert byte equality with `connection_id` and `app_version` masked and nothing else; `status.protocol` is `1`. Also run the existing `tests/bridge/*` suite unchanged. A v1 hello carrying `profile` is `Unsupported/profile_requires_protocol_2`; a v2 hello without one is `profile_required`. |
| A pushed row for an image message carries kind, MIME type and size, and `get_attachment` returns a readable file under the attachments grant | Host integration (PR 4) | Local homeserver, send an image to the agent DM; the pushed row has `message_kind:"image"`, `mime_type`, `size_bytes`; `get_attachment` with the profile's `attachments` grant returns a path inside the media cache that exists and is readable; without the grant it is `consent_required`/`not_permitted`. A pre-field row pushes with the fields absent. (The Core unit test that intake records the fields and an old log still loads belongs to #2131.) |
| A protocol-version mismatch fails with a typed error in both directions | Core unit + host integration (PR 1a) | Host: hello with protocol 0 and 3 after a valid token gets `unsupported/protocol_version` with `supported:[1,2]`; with a wrong token still `unauthorized`. Client (`koushi-mcp` unit, fake server over duplex): a server answering the legacy untyped `unsupported`, a server answering another protocol in `hello_ok`, and a typed `Error` each map to `Unsupported/protocol_version`. An invalid client name is rejected by the launcher before any socket is opened, and a host answering `Unsupported/client_name` is not reported as a version problem. The launcher sends `1` without `--profile` and `2` with it. |
| Headless UI test for creating, revoking and showing the status of a profile | Headless UI (PR 3) | Fake backend that serves a `McpAssistantsState` with `profiles`. Create with an agent account (and: a non-agent account is not offered); the snippet text is exactly the Rust-built string and the secret never appears in the DOM; `last_rejected` is shown; the Tauri capability test of PR 3 shows remote content cannot call create/revoke; status transitions `ready` → `connected` → `subscribed` follow state updates; revoke asks for confirmation and removes the row. A Rust unit test pins the `ProfileView` projection and the snippet builder. |

**Subject isolation, two directions (PR 1b).** Fixture: a client-name grant
`("Claude Code", agent A, read+send)` in `connections.json`, and a profile `P` bound
to agent A with different grants.

1. *Client grant never lets a profile through.* A profiled connection whose hello
   `client.name` is `"Claude Code"` and whose profile has **no** grants gets
   `consent_required`/`not_permitted` for every tool, despite the matching name
   grant. Specifically: `list_accounts` returns the bound account with **P's**
   permissions (none), not the name grant's; `read_recent_activity` is refused;
   a cursor minted for `Client("Claude Code")` by `read_recent_activity` is rejected
   by `activity_pages` when presented by `Profile(P)` and the reverse; a
   `send_message` idempotency key used by the client is not replayed for the
   profile (`DirectRetryKey`); a draft created by the client is invisible to the
   profile via `get_draft`; a pending or denied approval for the name does not
   affect the profile, and `remove_client("Claude Code")` does not touch the
   profile's approvals.
2. *Profile grant never lets a client-name connection through.* A connection
   with no profile and the name `"P"` (the profile's label, and also its raw id
   as a name) gets none of P's grants and sees none of P's account in
   `list_accounts`; a name-grant for a different name is unaffected by granting P.
3. *Same-account profiles stay distinct.* Profiles P1 and P2 bind agent A; granting
   `send` to P1 leaves P2 without it; their approvals, idempotency keys, activity
   cursors and drafts do not cross.
4. *Type guard.* A compile-fail (trybuild) test that building any grant,
   consent, retry, activity-page or draft key from `ClientIdentity.label` does not
   compile.

**Per-tool foreign `account_id` table (PR 1b).** For profile A (bound to agent A),
calling each of `search_messages`, `read_recent_activity`, `get_attachment`,
`draft_message`, `get_draft` and `wait_for_events` with `account_id` = agent B,
the owner, and a nonexistent id, returns the identical
`not_permitted/profile_account`, before any authorisation and with no data and no
difference in shape or timing class between an existing and a missing account. The
same call with `account_id` omitted is an invalid-params error (the field stays
required), not a resolution to A. The table is generated from the dispatcher's tool
list, so a new tool without a row fails the test.

**Authentication (PR 1b).** Wrong secret, unknown id and malformed secret return
the identical error; the unknown-id path runs the dummy verifier; no lockout after
many failures and the real plugin still authenticates; `last_rejected_ms` is set
and shown. Secret-file creation: `O_EXCL` refuses an existing path; created mode is
`0600` with no window in which it is wider; crash between secret and verifier
leaves an orphan that authenticates nothing and is swept; revoke deletes the row
before the file; the launcher refuses a wrong-owner, group-readable or symlinked
file. Concurrent: an owner revoke racing a second writer that re-reads under the
lock never resurrects the profile; an ack flush does not rewrite `profiles.json`
(file mtime and bytes unchanged).

**Revoke in flight (PR 1b/2).** A long poll and a pending approval end with
`unauthorized/profile_revoked` and return no data; a `send_message` already in
progress is documented as completing, and the test asserts the next request is
refused.

Additional tests that carry the security claims:

- `profiles.json` fail-closed: missing, malformed, newer schema grant nothing and are
  never overwritten (copy the `grants_tests.rs` cases).
- No IPC method writes the store: a table test calls every method name the dispatcher
  knows with a profiled connection and asserts the bytes of `profiles.json` are
  unchanged (the cursor file may change on `ack`; it is a different file).
- The secret never appears in a diagnostic, snapshot DTO, `Debug` output or error
  message (`ProfileHello` and the profile row implement redacting `Debug`).
- Role removal turns a ready profile into `not_agent_account` and ends its subscription.
- Revoke while subscribed ends the stream and releases the lease.

## 11. Open points for the reviewer

1. **Profile secret outside the OS secret store (D4).** The case rests on Key
   Management governing the at-rest unlock key, not authentication credentials, and on
   a same-user attacker already holding the `endpoint.json` token. The secret does
   authorise decrypted reads and sending, so this is a real credential. If the
   reviewer disagrees, the alternative is to keep the secret in the Keychain on macOS
   and a file on a headless host, which makes this stage depend on the open headless
   store-key decision.
2. **Unprofiled access to agent accounts.** This design leaves it as today. Closing it
   (for example refusing client-name grants on agent accounts) is a behaviour change to
   existing MCP and is not proposed here.
3. **Limits** (`PUSH_WINDOW`, `ACK_STALL`, `WRITE_STALL`, ...) are starting values, not
   derived; see "Constants" near the top. They are not API.
4. **Canon edits** land with the implementing PRs (profile paragraph and the grant-subject
   rule in PR 1b; the Key Management clarification that a profile secret is an
   authentication credential, not an unlock secret, in PR 1b; ack and lease in PR 2).
5. **Same-user isolation** does not exist (section 2). The canon and the UI say
   "limits what an assistant may do", and a stronger claim would need the agent to
   run as a different OS user or in a sandbox, which is out of scope.
