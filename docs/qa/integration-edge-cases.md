# Integration Test Edge-Case Matrix

Source: GitHub issue #31. This file is the repository mirror used by #9/#31 so final integration coverage can be reviewed and versioned with code changes. Keep issue #31 and this document synchronized.

## Purpose

The **final integration-test matrix** for umbrella #12 and the later roadmap
hardening issues now tracked by #70 — concrete, testable edge cases mined from
Element Web / Desktop / Element X GitHub issues, their unit/integration tests,
and Matrix spec/MSC gotchas. Each feature family wires its own Rust/headless or
browser-headless check; this matrix is the *exhaustive* end-state pass executed
under #9/#31 before pre-dogfood audit closure.

**Legend** — verifiable: `[H]` headless (Rust-owned state / mock-IPC / browser-DOM) · `[N]` native (real rendering, scroll, OS surface) · `[HN]` both. Refs: `EW#` element-web, `RS#` matrix-rust-sdk, `JS#` matrix-js-sdk, `EX` element-x, `MSC`/`spec`.

> Most items are assertable headlessly against Rust-owned state via crafted `/sync` fixtures + command/IPC assertions. The `[N]`/`[HN]` subset (scroll anchoring, blurhash/thumbnail layout, RTL mirroring, IME, OS badge/toast, fonts/emoji) needs the Linux virtual-display lane.

---

## 1. E2EE & device trust (→ #13)
- [ ] [H] to-device `m.room_key` un-drained when sync token commits early → key re-delivered, no permanent UTD (EW#23113)
- [ ] [H] pre-join messages classified `SentBeforeWeJoined`, tiles **hidden not removed** so they re-render when history keys arrive (EW#16983); same in threads (EW#27577)
- [ ] [H] historical-message UTD routing across the {backup exists × configured × device verified} matrix (RS utd_cause)
- [ ] [H] successful re-decrypt on key arrival fans out to replies, thread summaries, pinned previews, permalinks (EW#13473, RS#4196/#5703/#5798)
- [ ] [H] withheld `m.unverified` → `WithheldForUnverifiedOrInsecureDevice`; other codes → `WithheldBySender`; `m.no_olm` self-heals via fallback key (RS#3524/#281)
- [ ] [H] key arrives before UTD persisted → retry still converges, never permanent UTD (RS#5474)
- [ ] [HN] Olm session wedge recovery via `m.dummy`, rate-limited 1/hr/device (EW#7428, MSC1719)
- [ ] [N] reject `m.room_key` with mismatched embedded identity keys (CVE-2025-48937)
- [ ] [H] backup readiness terminates when decryption key absent (no infinite 'out of sync' loop) (EW#29872/#29141)
- [ ] [H] non-default 4S key id fetched correctly; distinct error for missing-secret vs wrong-recovery-key (EW#29553/#27458, EX-android#5099)
- [ ] [H] backup version bump / remote-delete invalidates cached version+key before restore (EW#22036/#26535); validate `auth_data.public_key` not version id (EW#7448)
- [ ] [H] base58 recovery key round-trips cross-client incl. whitespace + parity byte (element-ios#3470, EW#16243)
- [ ] [HN] large backup restores incrementally with progress; **failed restore must never mutate server backup** (EW#23359, EX-ios#1976)
- [ ] [H] SAS glare: lexicographically-smaller user/device id wins; both converge to one session (EW#20773, MSC1717)
- [ ] [H] verification request future/past `origin_server_ts` skew ignored; dual 10-min/2-min timeout → auto `m.timeout` (MSC1717)
- [ ] [HN] full cancel-code matrix lands BOTH peers in terminal `cancelled` (incl. `m.mismatched_sas`, `m.key_mismatch`) (spec SAS)
- [ ] [HN] QR reciprocate verifies the **displaying** device; correct direction label (EW#21853)
- [ ] [H] reject device-id shaped like a cross-signing key during MAC (CVE-2022-39250)
- [ ] [H] new-login backup key via secret gossip auto-triggers bulk re-decrypt (not only the passphrase path) (EW#26312/#27009)
- [ ] [H] reset identity is atomic — new private CSKs survive a concurrent `/keys/query` (RS#4728); no 'you reset your identity' warning about **self** (EW#30505)
- [ ] [H] reset deletes stale 4S/backup before generating a new recovery key; offers only valid methods (EW#29133)
- [ ] [H] identity-change notice fires on verified→unverified transition (RUSTSEC-2024-0434); benign re-login distinguished where possible (RS#4099)
- [ ] [H] one trust state drives header/room-info/profile/session-list shields consistently (EW#29160)
- [ ] [H] per-event shield = author device cross-signed, independent of per-user trust (EW#19561)
- [x] [H] eligible unverified peer devices remain non-blocking for ordinary sends; no normal-mode verify/send-anyway prompt. Explicitly blocked devices are withheld the room key while the event still reaches nonblocked devices (#191; Synapse tokens `e2ee_unverified_peer_send_nonblocking=ok` and `e2ee_blocked_device_withheld=ok`). Cryptographic send failures remain typed `TimelineFailureKind` failures and verification key mismatches remain closed `MatrixVerificationCancelKind::KeyMismatch`; neither path is converted into a send-anyway prompt. The vendored SDK's `MegolmError::MismatchedIdentityKeys`, `InvalidSignature`, and key-mismatch branches return errors rather than plaintext/partial results, while Core's timeline classifier propagates non-send-queue SDK failures as `TimelineFailureKind::Sdk`. A live mismatch injector is intentionally absent because safely corrupting persisted device identity keys is not a supported product or QA operation.
- [ ] [H] provisional login method discovery reaches `AwaitingVerification` on a disposable server before gate acceptance. Unknown trust remains fail-closed while authoritative identity facts may still expose only the applicable proof/bootstrap capabilities.
- [ ] [H] late-decrypt within grace window not reported as UTD; permanent UTD reported once even if timeline dropped (RS#4267, EW#25816)
- [ ] [H] megolm rotation reshares to the **full** current device set, excludes `Withheld` devices from oversharing/rotation (JS#1986, RS#4954)

## 2. Sync & timeline core (→ timeline core, #15, #16, #23)
- [ ] [H] initial state block = timeline start state; incremental never refetches full state (spec)
- [ ] [H] `timeline.limited` → treat as GAP, drop in-memory continuity, require backfill from `prev_batch` before claiming contiguity (RS PR#4694, JS)
- [ ] [H] limited-sync `state` delta (membership/topic in the gap) applied at gap boundary; flush member cache on `limited` (MSC4186)
- [ ] [HN] back-pagination to room start renders 'start of room', stops (no infinite loop)
- [ ] [N] **scrollback anchor stability** — prepend keeps the focused event fixed; reserve space for late images/blurhash/URL-previews (EW#8565/#20341)
- [ ] [HN] forward-paginate joins live edge with no duplicate boundary event (element-android#862)
- [ ] [H] dedup same `event_id` from /sync vs /messages vs federation; back-then-forward seam appears once (JS, RS Deduplicator, SYN-775)
- [ ] [H] order by stream/topological order, NOT `origin_server_ts` (clock skew) (JS#3325)
- [ ] [H] 'all events are own echoes' does NOT cancel a gap (RS PR#6190)
- [ ] [HN] local echo replaced in place via `unsigned.transaction_id`; remote echo before HTTP response self-heals (JS#2618); guest w/o txn id still single render (EW#13706)
- [ ] [H] send-queue strict FIFO; A fails, B/C queue behind; retry order preserved; no later receipt before earlier pending (RS#6016, EW#18942/#29677)
- [ ] [HN] retry after WiFi drop actually re-sends; banner clears (EW#29126/#26498)
- [ ] [HN] unsent events survive app restart, re-sent in order (RS respawn)
- [ ] [HN] cancel/abort in-flight upload: no orphan tile, object URL freed (EW#25756/#14881/#28302)
- [ ] [HN] 429 `M_LIMIT_EXCEEDED` backs off `retry_after`, clamps zero/negative (JS, synapse#9331)

## 3. Edits / redactions / replies (→ #19)
- [ ] [H] edit arriving before target buffered, applied on load (EW#18325)
- [ ] [H] latest edit by highest `ts`, tie by greatest `event_id`; edit by different sender ignored; edit of redacted ignored (MSC2676)
- [ ] [HN] edited reply keeps reply relation (new_content drops m.relates_to) (EW#13811/#22436)
- [ ] [H] encrypted edit: relation cleartext, new_content encrypted (MSC2676)
- [ ] [HN] redact message+all its edits → single tombstone, no edit content leak (EW#10191)
- [ ] [H] redaction before target buffered; belated target never flashes un-redacted (spec)
- [ ] [HN] redact others requires PL ≥ redact AND ≥ `events[m.room.redaction]` (EW#22606); self-redact always allowed (EW#22262)
- [ ] [HN] redacted media no longer retrievable from cache (synapse#17710)
- [ ] [HN] reply quotes only immediate parent (strip nested `<mx-reply>`); reply to redacted parent shows 'deleted' not cached text (react-sdk#8016, element-android#2236)
- [ ] [HN] reply to edited parent shows edited content (EW#22436); unknown parent lazily fetched, placeholder not error (EW#13380)
- [ ] [H] parse reply relation from `m.relates_to` only; treat `<mx-reply>` as untrusted (MSC3676)

## 4. Threads (→ threads in #19/#23)
- [ ] [H] accept both `io.element.thread` (legacy) and `m.thread` (EW#27332)
- [ ] [HN] thread fallback `m.in_reply_to` targets last thread event, not root; not rendered as reply unless `is_falling_back:false` (MSC3440, EW#23147)
- [ ] [H] room reply-count and thread-panel count agree incl. fallback replies (EW#23952/#19910)
- [ ] [H] deferred thread replies after own txn-id reply in same batch not dropped (JS#3665)
- [ ] [H] main message retroactively becoming a thread root: thread created, root not counted as reply (MSC4037)
- [ ] [HN] reaction on thread root shows in both main timeline and thread view (EW#19638)
- [ ] [H] room read only when all threads + main read (threaded receipts) (EW#25623)
- [ ] [H] per-thread highlight/notification counts sum to room total, no double-count (MSC3773, EW#26233)
- [ ] [H] thread-root annotation receipts attributed to main timeline; each event in exactly one thread (MSC4037, EW#24392)

## 5. Reactions (→ #8)
- [ ] [HN] double-react same key dedups; server 400 `M_DUPLICATE_ANNOTATION`; no phantom echo (EW#13586)
- [ ] [HN] normalize VS16 — `👍` vs `👍️` aggregate as one (EW#27331)
- [ ] [H] dedup by sender per key across federation (MSC2677)
- [ ] [HN] redact reaction decrements count, drops from who-reacted; pill gone at zero (MSC3912)
- [ ] [HN] reaction to later-redacted event dropped; clears any thread unread it caused (EW#26388)
- [ ] [H] reaction before target buffered; `Relations` ordered by timeline not insertion
- [ ] [H] render reactions from bundled `unsigned[m.relations]` aggregation (matrix-viewer#8)
- [ ] [HN] custom image reaction (MSC4027 mxc key) aggregates, degrades to shortcode, never crashes (EX-android#2159)

## 6. Media (→ #15)
- [ ] [HN] ciphertext SHA-256 mismatch → integrity error distinct from crypto-key failure; no partial render (EW#28312)
- [ ] [H] malformed encrypted `file` (v≠v2, alg≠A256CTR, missing key/iv) rejected without AES attempt (MSC1420)
- [ ] [HN] thumbnail vs full-file decrypt/verify independently (EW#28312)
- [ ] [N] blurhash placeholder sized to info.w×h, swapped for real thumb, no layout shift; tolerate garbage blurhash off main thread (MSC2448, EW#17945/#18411)
- [ ] [HN] thumbnail ≥ original → use original; clobber w/h on animated-GIF swap (EW#17906/#1071)
- [ ] [HN] MIME spoof (HTML/SVG as image/png) forced safe content-type + nosniff, sandboxed/forced-download (XSS class)
- [ ] [HN] check `m.upload.size` before upload; handle 413 mid-flight, clean local echo (RS PR#5119, EW#21091)
- [ ] [HN] sanitize download filename (path traversal / RTL override in `body`) to bare basename (matrix-android-sdk#228)
- [ ] [H] authenticated media (MSC3916) first; fall back to legacy on `404 M_UNRECOGNIZED`; never leak token (Matrix v1.11)
- [ ] [HN] voice-message waveform clamped/tolerant of empty/over-long/out-of-range samples (MSC3245)

## 7. Composer: markdown / mentions / shortcuts (→ #18, #6)
- [ ] [HN] very long message / huge code block / nested markdown renders without freezing (off-main-thread) (EW#21127)
- [ ] [HN] literal `>>>` / `~~~` / fence edge cases round-trip; markdown-off respected (EW#28474/#4931)
- [ ] [HN] quoting a code block preserves contents, no stray backslashes (EW#7324)
- [ ] [HN] mention pill for unknown/left user falls back to MXID; intentional mentions via `m.mentions` not raw-text (MSC3952)
- [ ] [HN] sanitize inbound `formatted_body` to allowed HTML subset, neutralize scripts/`javascript:`/mxc-spoof (XSS)
- [ ] [HN] 'Enter sends' vs 'Ctrl/Cmd+Enter sends' modes; consistent across main/thread/edit composers (EW#11322)
- [ ] [N] Enter during active IME (CJK candidate window) commits, does NOT send
- [ ] [N] Cmd+Enter (macOS) vs Ctrl+Enter (Linux/Win) logical 'send'

## 8. Membership / invites / DMs (→ #14)
- [ ] [H] accept invite to encrypted room → post-join messages decrypt (reshare); pre-join UTD unless history key-share (EW meta#245, MSC3061)
- [ ] [H] reject invite when inviter HS offline still clears locally; declined invite stays gone after re-sync (EW#4225/#3743)
- [ ] [H] invite to already-joined room = clear no-op (EW#8965); retracted invite (404) forgets room (EW#29006)
- [ ] [H] invite from ignored user suppressed for NEW rooms (asymmetry vs known rooms) (synapse#18209, spec)
- [ ] [HN] invite preview from stripped `invite_room_state`; peek 403 stops spinner with affordance (EW#12500/#1243)
- [ ] [H] decline never depends on room-state/summary read (403 for non-joined) (EX-ios#3713)
- [ ] [H] re-invite after reject/ban reappears & is acceptable; re-invite of banned blocked until unban (EW#22106/#7362)
- [ ] [H] knock issues POST `/knock` not join; knock→invite recognized; rejection renders readable, not UTD preview (EW#27025/#27659, MSC2403)
- [ ] [H] join by alias resolves servers; by id needs via-hints; matrix.to `?via=` forwarded (EW#22845/#15100)
- [ ] [HN] join failures surfaced: M_NOT_FOUND alias, 403 not-invited, make_join fail, 429 backoff (EW#25562/#224/#17740)
- [ ] [H] forget room in one action, gone after sync+restart (EW#27667)
- [ ] [H] kick rendered 'Y removed X' w/ reason ≠ 'X left'; 'joined and was banned' summary (EW#7853/#23588)
- [ ] [H] `is_direct:true` alone classifies DM (no non-spec data) (EW#14046); reuse existing DM, dedup, self-DM keyed by valid MXID (EW#15075/#24781, RS#5043)
- [ ] [H] `m.direct` never rewritten on typing/composer churn; never clobbered to {} on half-wiped store (EW#24059/#3610)
- [ ] [H] left DM pruned from DM lookup; recreated DM repopulates People tab across sync/account-data ordering (react-sdk#9880, EW#29482)

## 9. Rooms / upgrades / power levels / state (→ #21, #11)
- [ ] [HN] follow tombstone to replacement **live edge** not create event; validate bidirectional predecessor link (EW#14532)
- [ ] [H] upgrade 'invite all members' re-invites every member, lossless (EW#31395)
- [ ] [HN] tombstoned predecessor hidden from list/spotlight/favourites after rejoin+restart (EW#28179/#20141)
- [ ] [H] multi-upgrade chain (A→B→C) dedupes/bounds; View/remove any room in chain (EW#21732)
- [ ] [HN] redact button hidden when PL insufficient (EW#22606); can't grant PL above self; warn self-demote/last-admin (EW#23882/#2855)
- [ ] [H] explicit power_levels honored; spec(50) vs Synapse(0) invite default split-brain (matrix-spec#1019)
- [ ] [H] no-op PL change emits no event (EW#5678); PL change attributed via prev_content after upgrade (EW#30560)
- [ ] [H] name from canonical_alias→heroes (never `m.room.aliases`); 'Unnamed' vs 'Empty' (members left) (EW#22263/#7323)
- [ ] [H] other-member avatar fallback only for 2-person DM; disambiguate only real displayname collisions (react-sdk#6895, EW#5914)
- [ ] [HN] tile/header re-derive on new `m.room.name`/avatar state, not stuck (EW#17480)
- [ ] [H] soft-failed / state-reset reconciliation deterministic, no crash (state-res v2, synapse#15987)

## 10. Directory / spaces (→ #20)
- [ ] [H] join from directory via alias/server_name, not bare id (EW#23937/#22845)
- [ ] [H] remote directory not federating → 'server not sharing directory' not 'internal error' (EW#15359)
- [ ] [H] `/publicRooms` paginated (since/next_batch); no unpaginated federated fetch (EW#3327)
- [ ] [H] tombstoned room filtered/marked in directory/spotlight (EW#20141)
- [ ] [HN] room in multiple spaces: no spurious back-and-forth navigation (EW#17017)
- [ ] [H] cyclic space membership dedupes visited, bounds traversal (MSC2946)
- [ ] [H] space hierarchy expired pagination token transparently restarts (EW#22138, synapse#17340)
- [ ] [H] suggested rooms (`suggested:true`) appear live (EW#18761)
- [ ] [H] restricted join rule (MSC3083 allow) evaluated; 'Unknown room' fallback for unloaded allow target (EW#18780)
- [ ] [H] space child order: codepoint compare, ties by child create ts then room_id (EW#19192, MSC3610)
- [ ] [HN] leave space vs leave space+rooms distinguished, enumerates affected, least-destructive default (EW#18592)

## 11. Tags: Favourites / Low priority (→ #22)
- [ ] [H] room appears once under current tag; promote/demote moves not duplicates (EW#14508)
- [ ] [H] `order` float [0,1], NO spec tie-break → deterministic secondary key for equal/missing order (spec, react-sdk ManualAlgorithm)
- [ ] [H] midpoint-insert float exhaustion → rebalance, never collapse distinct rooms (EW#6369)
- [ ] [H] tag namespaces: m.* reserved, u.* user, tld.* private (spec)
- [ ] [H] room simultaneously m.direct AND favourite/low_priority (orthogonal buckets) (spec)

## 12. Receipts / markers / counts (→ #16, #10)
- [ ] [H] public + private read receipts stored separately; never leak `m.read.private` to others; own pos = latest of either (spec)
- [ ] [H] server drops private-receipt support mid-session → graceful fallback, not stuck (EW#23433)
- [ ] [H] receipt for not-yet-loaded event buffered; conflicting devices → max position, never backwards (EW#21016)
- [ ] [N] 'show others receipts' off honored incl. threads (EW#24910)
- [ ] [H] `m.fully_read` only advances forward; ignores stale lower marker (EW#20026)
- [ ] [HN] marker steps past hidden/redacted events consistently; jump-to-unread lands on first VISIBLE unread (EW#9889/#12338)
- [ ] [H] leave-within-debounce flushes pending fully-read (EW#20026)
- [ ] [HN] mark-as-read advances main + all thread receipts; thread dots clear (EW#24229)
- [ ] [HN] badge not optimistically cleared if receipt RPC fails (no reappear on refresh) (EW#25611/#23754)
- [ ] [H] highlight vs notification vs unread are 3 distinct values; never conflated (spec)
- [ ] [HN] space/home badge == exact sum of room counts (EW#27882); counts hit exactly 0, never negative/stuck (EW#24595)
- [ ] [HN] thread reply from other client yields room count+dot (EW#25621); read-elsewhere clears stale thread dot (EW#23770)
- [ ] [H] edit and reaction do NOT bump unread/notification; redaction of highlight decrements (EW#9745/#26388)

## 13. Typing / presence (→ #16)
- [ ] [HN] typing auto-expires at `timeout` (~30s) with no refresh; stale typing clears on disconnect (TTL class)
- [ ] [HN] send implies typing-stop, indicator clears on the message; own typing never shown to self
- [ ] [H] throttle outbound `m.typing` (not per keystroke); typing EDU with room_id never notifies (EW#17263)
- [ ] [HN] presence disabled on HS → neutral 'unknown', not everyone 'offline' (EW#15485/#22548)
- [ ] [H] apply newest presence by last_active_ago; older 'offline' never overrides newer 'online' (EW#28738)
- [ ] [HN] user-disabled presence sharing stops own publish, still renders others (EW#31610)

## 14. Notifications / push / privacy / badges (→ #10)
- [ ] [HN] suppress own messages; suppress focused room while window focused; suppress initial-sync/backfill storm (EW#469)
- [ ] [H] keyword rule whole-word, case-insensitive; rule precedence override>content>room>sender>underride stops at first match (spec)
- [ ] [HN] per-room mute = zero notifications; `@room` toggle per-room/global honored (EW#26332/#12769)
- [ ] [H] DM default notify, large room mentions-only (`.m.rule.room_one_to_one`) (spec)
- [ ] [H] push decision uses decrypted content; undecryptable → generic `.m.rule.encrypted` (spec)
- [ ] [HN] 'hide content' → generic toast, no body/sender/room; encrypted toast never shows ciphertext (privacy)
- [ ] [H] push-gateway payload uses `event_id_only` when privacy on (no body to sygnal/Apple/Google) (spec)
- [ ] [HN] OS dock/taskbar badge == in-app aggregate; clears to none (not '0') at zero (EW#15967/#27882)
- [ ] [N] toast click focuses correct room/thread, brings window front; coalesce bursts; respect OS DND (EW#28418)
- [ ] [HN] sanitize bidi/control chars in sender name in toast (spoofing)

## 15. Settings persistence / migration (→ #6)
- [ ] [H] corrupt/partial settings blob → per-key defaults, valid keys kept, no crash (EW#18258)
- [ ] [H] versioned migration runs once; downgrade-then-upgrade safe; level precedence device>room-account-data>account-data>config>default
- [ ] [H] concurrent multi-device account-data settings: last-write by revision, reconcile on sync; quota-exceeded caught, warned, no false success (EW#3660/#26605)

## 16. i18n / RTL / fonts / emoji (→ #4, #5)
- [ ] [N] Arabic/Hebrew mirrors whole layout via logical props; embedded LTR (URL/code/mention) isolated (FSI) (W3C bidi)
- [ ] [HN] per-string `dir=auto` so one RTL name doesn't flip LTR layout; runtime locale switch re-renders direction+formats live
- [ ] [HN] missing catalog key → source fallback, never blank/undefined; locale plural categories (Arabic 6, Polish few/many) (ICU)
- [ ] [N] pseudo-locale +30–40% no truncation/overflow on buttons/badges/menus
- [ ] [N] CJK line-break, combining marks & ZWJ render as single grapheme; cursor by grapheme
- [ ] [N] emoji fallback chain (no tofu); COLR/CBDT color, skin-tone & ZWJ families single glyph; VS15/VS16 text-vs-emoji presentation
- [ ] [N] missing-script glyph falls back, `.notdef` only as last resort, no crash

## 17. Profiles / avatars (→ #17)
- [ ] [HN] per-room displayname/avatar overrides global for that room (spec)
- [ ] [HN] no flicker to MXID when profile momentarily unresolved (cache last-known) (EW#31708)
- [ ] [H] avatar `/thumbnail` requested at rendered px × DPR with method, cached by size key (not full-res)
- [ ] [HN] no-avatar → grapheme-aware initials + stable per-user color, consistent across views (EW#2449)
- [ ] [HN] profile/avatar change propagates to timeline/member-list/header/room-list without reload; cleared avatar reverts to initials
- [ ] [N] mxc from unreachable HS → initials fallback, no broken-image, retry/backoff

## 18. Account-wide activity view (→ #23)
- [ ] [H] Recent/Unread merge & ordering computed in Rust-owned state, recency-window vs per-room-unread bounds (stale unreads still surface)
- [ ] [H] muted/low-priority excluded from both streams; viewing Unread does not auto-mark-read; jump marks read; mark-all clears
- [ ] [HN] click row opens focused context to that event; live updates prepend/evict correctly

---

## Execution under #9
- These run as the integration/walkthrough lane after each feature's own GUI-operation check is green.
- `[H]` items: extend the Rust headless core QA + browser-headless (Vitest/Playwright) suites with crafted `/sync` fixtures; private-data-free tokens.
- `[N]`/`[HN]` items: Linux virtual-display lane.
- Keep this document and GitHub issue #31 synchronized whenever the final matrix changes.

## Current triage for the pre-dogfood audit

Status date: 2026-06-18.

This section is the current #31 contract for the #9 automated audit pass. It
does not mark every matrix row above as complete. "Covered" means the standing
gate exercises the current shipped surface for that family with Rust-owned state
or typed browser/headless commands. Row-level edge cases above remain open until
a focused fixture, browser-headless check, or native lane is added for the exact
case.

Covered by standing automated gates:

- Rust core/local-homeserver QA now covers login/sync, credential health,
  native-attention state, invites and DM start, room/space creation and joins,
  public directory query/join, room settings and moderation guards, timeline
  send/edit/redact/pagination/navigation, activity streams, composer
  mention/markdown/slash/IME guards, read receipts, fully-read markers, typing,
  presence, reply quotes, pins, media staging/gallery/compression, link
  previews, threads, scheduled send local fallback, send queue retry/cancel/FIFO,
  restart persistence, search edit/redact, and E2EE trust/key-backup/
  verification/identity-reset smoke.
- Browser-headless gates cover visible-control dispatch and Rust-shaped
  snapshots for create room/space, invites/DMs, directory, activity, room
  management, room tags, notification attention, aliases/profile labels,
  mention and markdown composer paths, send queue, reactions, reply/pin/source/
  forward actions, media/file/link-preview surfaces, live signals, search,
  threads, locale/RTL/CJK/pseudo-locale, typography/font tokens, settings,
  security, device sessions, reports, and scroll/navigation surfaces.
- Structural gates cover IPC contract drift, TypeScript typecheck, wasm
  compilation for state/search crates, secret scanning, release-gate structure,
  QA token privacy checks, and Rust/Tauri focused contract tests.

Failures found in this pass and fixed in the same hardening branch:

- Browser-headless display-setting expectations were stale after URL-preview
  state became part of the Rust-owned display DTO; the tests now assert the full
  Rust-shaped display patch.
- The link-preview local-homeserver QA waited for a transient
  `SettingsPersistenceState::Saving` snapshot. Fast local persistence may reach
  final `Idle` before publication, so the QA contract is now final `Idle` plus
  the expected URL-preview policy value.
- Per-room URL-preview overrides were removed from persisted `SettingsValues`
  because their keys are Matrix room identifiers. They now live in non-persisted
  Rust-owned `AppState.link_preview_settings` and are changed through a typed
  room-override command.
- Public boundary `Debug` is now explicit repository policy: derive only when
  every field is artifact-safe; otherwise implement redacted `Debug` exposing
  only kinds, booleans, counts, lengths, request ids, and placeholders.

Open follow-ups:

- #74 tracks browser fake top-level timeline/thread DTO alignment so
  browser-headless evidence stays congruent with the Tauri/CoreEvent path.
- #75 tracks mention autocomplete/member ordering that still needs stronger
  Rust-owned text/order semantics, especially for CJK/profile confidence.
- Native-only and attended residual evidence stays in #66/#67. Product rename
  issue #82 (Koushi) must run before macOS/native smoke so native evidence uses
  Koushi naming.

Deferred from this automated pass:

- Exact row-level fixtures for the full E2EE/device-trust matrix, deep sync-gap
  behavior, room upgrades, power-level edge cases, tag ordering/rebalancing,
  settings multi-device conflict resolution, and many Element-derived
  historical bug regressions above.
- Native rendering and OS-surface checks that need a real WebView or platform
  adapter: scroll anchoring with real media layout, blurhash/image layout shift,
  desktop toast click/focus, dock/taskbar badges, fonts/emoji fallback, CJK IME
  candidate handling, macOS Keychain prompts, and platform notification
  compatibility.
