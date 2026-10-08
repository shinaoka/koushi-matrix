# #1150: index-first search (M2)

Status: implementation and final review on PR #1157. SDK changes are in fork
PR #19; that PR must not be merged without a separate maintainer decision.

## Scope and acceptance

The encrypted persistent ngram index is the only search candidate source.
Verification reads current visible content from the encrypted SDK event cache;
ngram false positives, redacted content and content excluded by the account's
policy are never search results. Existing UTF-16 highlight and normalization
semantics remain in the synchronous, pure `koushi-search` verifier.

First-party search residency must not grow with historical message bodies or
edit text. Files metadata remains resident; SDK/cache residency and index/disk
size are separate quantities, not covered by a zero-body-byte assertion.

The maintainer explicitly removed **M3** (durable crawl commitments and their
compensating Files reconstruction) and **M4** (startup search warm set). Restart
crawls history again. There is no warm-target persistence, startup warmer, or
startup-latency claim in this PR. The version-tagged index directory remains:
an extraction/tokenization change needs a fresh index because existing IDs are
not automatically rewritten.

## Implemented search boundary

- Literal, offset-free SDK paging, newest first by index `(timestamp, event_id)`.
  Verification refills filtered pages within 50-candidate pages and a
  500-candidate scan budget per query variant.
- Candidates resolve from the SDK encrypted cache, including replacement aliases
  and sticker text. Deduplicate resolved identity before the 50-result quota;
  select by the index paging key before sorting by displayed timestamp.
- Search and Files queries carry authoritative account content policy, bypassing
  deferred crawler notifications. Policy changes clear Files
  residency, close Search/Files state, and invalidate in-flight actor results.
- Full request identity includes connection and sequence. AppActor publishes
  successful search results only after reducer admission; superseded and failed
  requests settle through correlated outcomes.
- Crawler pages explicitly await persistent index commit before reporting
  success. Completed-room checkpoints are in-session only, not durable M3 data.
- `SearchDocumentStore` keeps attachment metadata, pending attachment relations,
  and body-free provenance for formerly attached messages replaced by text.
  Ordinary text messages and all edit bodies are discarded.

## Files redaction correction

Canon consulted: `REPOSITORY_RULES.md`, overview Search/async ownership,
state-machine Files View, engineering rules Search Index And Room-Key Export,
and verification discipline. SDK source is the checked-out vendor submodule.

Timestamp changes cannot prove an edit was redacted. Both the observation-counter
attempt and producer-local `reported_search_edits` ledger were removed: a stale
actor can report an older edit, a surviving superseded edit can later be promoted,
and actor replacement loses local history. Sharing that ledger would not make
its inference authoritative.

SearchActor instead checks actual SDK redaction evidence for attachment-affecting
mutations and before Files reads. A small fork cache API reads requested IDs
under one room-cache guard, including the SDK's existing pending-redaction map.
Live ingestion now remembers a committed redaction before returning for an absent
target; the same map is reconstructed from encrypted SDK storage. Missing events
are not redaction evidence: focused-only, bundled-edit and edit-before-root
payloads remain admissible through the ordinary edit-order guards.

Pending edits are rechecked before consumption, including root sender/room/type
validity: another user's replacement cannot mutate an attachment or poison its
edit ordering, even if it arrived before the root. Cached replacements use the
existing SDK validator, scoped by target (not every edit in the room), including
encrypted-root/plain-edit protection. Missing/UTD events stay unknown and do not
reject legitimate SDK projections. No encryption flags are added across DTOs.
Retirement removes only the
specified version, preserving other pending survivors. The bounded synchronous
store tombstone cache is not the arbitrary-replay guarantee: SDK evidence also
rejects already-superseded redactions and older IDs evicted from that cache.
Retired attachment targets retain body-free text provenance; a text replacement
arriving before a queued media root waits in the existing bounded queue.
Reconciliation seeds media before consuming text, without using server timestamps
as redaction proof. Both cases have RED→GREEN regressions. A same-sender
decrypted-root/plain-edit case also failed before the SDK validator was wired in
and passed afterward (incoming, applied and pending with a valid survivor).
A real SDK bundled timeline projection remains admitted when its edit is not
independently cached. The decryption-info fixture is not a crypto key-exchange
end-to-end claim.
Ordering is `(edit timestamp, edit ID, canonical tie-break)`, not canonical status
before edit ID. Text replacements remove the attachment while retaining no body;
the existing cache resolver recovers body-free text-replacement provenance when
a previous crawl page preceded the attachment target.

Content/relations reads propagate backend errors rather than treating them as
missing content. An encrypted SQLite regression holds the store lease while
closing the backend: the loaded root and positive-redaction lookup remain
readable, the relations lookup and resolver fail, and resolution recovers after
reopen. Redaction deletes both index primary-key and deletion-key terms, including
an edit whose own ID differs from its root ID (committed and same-batch tests).

A lookup error/timeout never means redaction. Body-free mutations await retry in
a bounded queue; input backpressure preserves distinct versions rather than
unsafe coalescing. One completed crawl page may wait separately, and no next
page starts while either queue is pending. The crawl work permit is released
before Files admission. A Files query drains retries and reconciles redactions
under one total deadline; failure preserves resident/retry state and settles the
existing Files failure transition rather than publishing unchecked rows.
A redaction committed after the read proof is handled by a subsequent SDK
projection/query, not an atomic SDK-to-renderer transaction.

This is not an M3 rebuild: the SDK projection still supplies promoted content.
No full-history metadata reconstruction, first-party plaintext store, actor edit
ledger, or network fallback is introduced.

## Verification evidence

- Headless RED→GREEN checks reproduce selective pending-edit retirement,
  equal-time stale canonical ordering, and media-to-text replacement in the pure
  Files store. Core regressions cover cross-producer rollback, root redaction,
  already-superseded/evicted-tombstone replay, pending consumption, missing cache
  coverage, recovery without payload resubmission, page ordering and Files-read
  redaction reconciliation.
- The actual SDK live-ingestion test reproduces redaction-before-target without
  reopening (RED before maintaining the existing pending-redaction map). A
  separate store-reconstruction test covers absent targets after reopening.
- `crates/koushi-search/tests/search_memory.rs`: 50,000 synthetic text messages
  leave zero resident body bytes and zero message rows; 5,000 attachment messages
  keep metadata but no message bodies. This is not a process-RSS or SDK-residency
  measurement.
- Explicit encrypted-index history probe (`TOKIO_WORKER_THREADS=1`, a verified
  current-thread async runtime, test harness `--test-threads=1`):

  | Indexed messages | Encrypted index bytes | SDK cache disk bytes | First-party body bytes |
  | ---: | ---: | ---: | ---: |
  | 1,000 | 157,161 | 5,308,168 | 0 |
  | 10,000 | 952,160 | 15,273,928 | 0 |
  | 50,000 | 4,524,622 | 58,998,752 | 0 |

  The latest 50 literal candidates are checked at every stage. SDK resident RAM,
  index-worker overhead, and process RSS are not measured; this is disk/residency
  evidence, not a 1T latency baseline or a total-process memory bound. The probe
  is `search::history_scale::encrypted_history_index_grows_without_resident_first_party_bodies`
  and is explicitly invoked with `--ignored --nocapture --test-threads=1`.
- Workspace tests (CI exclusions), the three Rust clippy lanes, frontend tests
  (1,726 passed), frontend typecheck/lint, and SDK search/cache focused suites
  passed after the corrections. Request-outcome coverage includes trimmed input
  and foreign-connection rejection. The targeted `search_crawler` headless lane
  passed on both Tuwunel and Synapse. An earlier aggregate Tuwunel core run hit
  its 240-second overall limit after the search/redaction stage; it is not a pass.
- Final aggregate local QA, the completed corrected-diff independent review,
  reachable SDK/app heads and required exact-head hosted CI are still merge
  gates. npm audit also reports an unrelated existing high `source-map-js`
  advisory: no frontend packaging/build is claimed while that gate is red.

## Remaining before merge

Complete the independent GPT-6.1 Sol post-review of the integrated correction,
resolve verified findings, run the affected repository gates and inspect all
required checks for the exact submitted head. Do not merge SDK fork PR #19 as
part of this operation.
