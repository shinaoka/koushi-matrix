# Nightly QA triage — 2026-11-01

## Scope and baseline

Read the bodies and comments of #1691, #1683, #1612, #1634, #1636, #1637,
and #1279 before running QA. GitHub state at that read: #1636 was already
closed after three nightly passes; the other six were open. No issue comments,
issue state changes, or workflow dispatches were made.

- Baseline after `git fetch origin`: `6fc090cf18289f72a5ae25b02136c995bb4ac8a2`,
  identical to `origin/main` when testing started. No product source was changed.
- SDK gitlink: `1c669ef107eba11b395efa275ed4f615a94c7c3b`, initialized in this
  worktree and accepted by `node scripts/check-sdk-submodule.mjs`.
- Host: local Apple Silicon Mac, arm64, macOS 26.7.2, 18 logical CPUs.
  No remote machine or other worktree was used. These were unpinned functional
  checks, not reproductions of the CPU-pinned Linux experiments in older comments.
- Servers: host Tuwunel 1.7.1 and the runner's pinned
  `matrixdotorg/synapse:v1.157.0` through the local Docker runtime. Every attempt
  created its own disposable fixture. No external homeserver or real account.
- Profile: `ci`, default scenario sizes and runtime threading. Elapsed seconds
  below include fixture startup and Cargo work; they are not benchmark results.
- `kache doctor --verify`: 32,749 entries valid, zero corrupt entries; no repair
  needed. QA binaries were prebuilt serially through kache with
  `KACHE_EXPLAIN_MISS=1`; core build exited 0 in 69 s, SDK build in 46 s.

Consulted contracts: [repository rules](../../REPOSITORY_RULES.md),
[environment](../agents/environment.md), [verification](../agents/verification.md),
[QA lanes](../agents/qa-lanes.md), and the documentation-only gate exception in
[engineering rules](../policies/engineering-rules.md#build-dependencies-qa-gates).

## Commands and evidence

Each matrix row below was run **serially**, at least three times, with this
command (substitute the row's server and scenario):

```bash
KACHE_EXPLAIN_MISS=1 node scripts/desktop-headless-local-qa.mjs --run \
  --server=<server> --core --scenario=<scenario> \
  --cargo-profile=ci --timeout-ms=600000
```

| Issue | Server | Scenario | Original failing stage |
| --- | --- | --- | --- |
| #1691 | tuwunel | `timeline_stress` | `timeout timeline_stress`, previously after `timeline_nav` |
| #1683 | synapse | `notification_delivery` | `notification delivery prefix destination unread` |
| #1612 | synapse | `gate_negative` | `gate negative trust loss reset failed` |
| #1634 | tuwunel | `all` | `crawl_throttle` |
| #1636 | synapse | `all` | `reader scope`, subsequently Space leave membership |
| #1637 | tuwunel | `history_key_recovery_window` | `history window: pending record missing before backdate` |
| #1279 | tuwunel | `user_verification` | `user_verification B incoming request` |

An additional three Tuwunel `search_crawler` runs used the same command shape
because the aggregate failed before reaching the crawler. These are a focused
control, not substitutes for passing aggregate runs.

Local, ignored evidence:

- `.local-secrets/nightly-triage/results.tsv`: issue, attempt, exit status,
  elapsed seconds. `1634-focused` identifies the additional crawler runs.
- `.local-secrets/nightly-triage/<issue>-<attempt>.log`: runner output.
- `.local-secrets/headless-local-qa/<stamp>-<server>-<scenario>-<suffix>/`:
  runner-validated `core-core-stdout.log` / `core-core-stderr.log`, SDK logs
  where applicable, and fixture data. The failed readiness attempt has no core
  log because it never started the scenario.

There are 25 attempts: 17 successful, 7 scenario failures, and 1 pre-scenario
failure. All successful runs' recorded core stdout was rechecked against
`requiredTokensForHeadlessScenario`, with explicit additional stress,
trust-loss, and crawler tokens. All three Synapse aggregate logs independently
contain `reader_scope_live=ok`. Only sanitized tokens and summaries are included
here; fixture data, generated credentials, server logs, and build logs are not
PR attachments.

## Verdicts: historical cause versus this baseline

`real-bug` includes defects in the vendored SDK used by Koushi. A timing-sensitive
product race is not automatically a `flake`; `server-side` requires evidence of
a homeserver-specific behavior. Three local passes alone establish neither a
fix nor a three-way cause classification.

| Issue | Historical class supported by issue evidence | Current local evidence | Disposition |
| --- | --- | --- | --- |
| #1691 | **flake**: QA edit-diff oracle/deadline race; #1693 is already on baseline | 3/3 full passes; original timeout absent | Existing correction corroborated, not a new fix |
| #1683 | **real-bug**: unread reducer compared against current rather than locally-cleared activity; #1689 is on baseline | 3/3 scenario passes; 1 additional startup failure | Existing fix corroborated; startup failure is separate |
| #1612 | **undetermined** | 3/3 full passes, including both trust-loss tokens | No local RED; do not call this fixed or a flake |
| #1634 | **flake**: old Completed/Queued oracle raced a valid catch-up; #1699 is on baseline | Aggregate 0/3 passes and 0/3 reach the crawler; focused crawler 3/3 passes | Original stage blocked in aggregate; new failures need separate diagnosis |
| #1636 | **real-bug**: reader publication ordering in Core (#1697), later leave ordering in SDK (#1698) | Reader passes 3/3; aggregate passes 1/3, with 2 later crawler failures | Original reader symptom absent; aggregate is not green |
| #1637 | **real-bug**: SDK skipped pending invite acceptance when sync joined first; #1695 is on baseline | Deferred/29-day/30-day proofs pass 3/3; full scenario 1/3, with 2 logout timeouts | Original pending-record symptom absent; cleanup is not green |
| #1279 | **undetermined**, product/SDK receiver path remains the issue's hypothesis | 3/3 full passes including incoming request, SAS, and verified identity | No deterministic receiver-path RED; do not infer resolution |

No **server-side** defect was established. #1612 and #1279 cannot honestly be
assigned one of the three requested cause labels from this experiment. Their
classification remains parked rather than fabricated. Existing historical
fixes were not reimplemented, and no new real-bug had a sufficiently isolated
root cause for a small product fix or an individual fix PR.

## Per-issue evidence and unposted comment drafts

### #1691 — timeline stress

Durations: 80, 18, 22 s; exits: 0, 0, 0.
Every run has `timeline_nav=ok`, `timeline_stress=ok`, `stress_no_blank=ok`,
`stress_space_scope=ok`, and `restore_cleanup=ok`.

The issue's failure-path diagnostics established an edited row arriving through
Insert rather than Set, while mutations had settled successfully. Baseline
contains `77b4d7d6` / #1693. The earlier Space-child timeout is a different
signature, also absent from these successful runs.

> Draft for #1691: On baseline 6fc090cf, the disposable local Tuwunel
> timeline_stress lane passed 3/3, including the navigation, edited/redacted
> stress, blank-row and Space-scope proof tokens. This corroborates the existing
> QA oracle correction in #1693; it does not replace the nightly streak rule.
> No additional product change was made.

### #1683 — notification delivery

Durations: 102, 15, 97, 97 s; exits: 0, 1, 0, 0.
The three completed scenarios contain every required notification-delivery
proof, including `notification_delivery_prefix=ok`,
`notification_delivery_prefix_consent_cleared=ok`, and
`notification_delivery=ok`. Baseline contains `22222045` / #1689;
`crates/koushi-state/src/reducer/room.rs` compares against
`locally_read_activity` in `suppress_stale_unread_after_local_read`.

Attempt 2 failed **before user registration/core execution**:
`Synapse Sliding Sync readiness probe failed`. Its retained server log had no
ERROR/Traceback entries, and an offline read-only query of its database found
three relevant migrations still pending. The failed probe's stderr/status
subtype is not persisted by the runner, so SQLite locking versus Docker exec
failure cannot be distinguished. This is a separate fixture/startup failure
(harness flake), not evidence of unread suppression or a homeserver semantics
bug. Its exact underlying trigger remains unproven. No container survived it.

> Draft for #1683: The Synapse notification_delivery scenario passed 3/3
> completed local runs on 6fc090cf, with all prefix/consent and delivery proofs.
> One additional attempt failed at the pre-scenario Sliding Sync readiness
> probe; it never exercised notification delivery. The unread-reducer fix in
> #1689 is already on this baseline. No claim of nightly closure is made.

### #1612 — gate negative

Durations: 172, 136, 130 s; exits: 0, 0, 0.
Every run reached `gate_trust_loss_regated=ok` and
`gate_trust_loss_commands_blocked=ok` after its cancellation/recovery proofs.

The reset is invoked by
`crates/koushi-qa/src/bin/headless_core_qa/scenarios/identity.rs`
(`reset_identity_for_qa`, trust-loss stage). The production owner is
`crates/koushi-core/src/account/recovery_backup.rs` (`handle_reset_identity`),
which calls `koushi_sdk::reset_identity` and projects the result. The SDK
adapter in `crates/koushi-sdk/src/e2ee.rs` delegates to the SDK recovery API.
Without a failing local result/failure kind, this does not distinguish product,
SDK/UIAA, timing, or server behavior. No retry or auth-policy change was made.

> Draft for #1612: The focused Synapse gate_negative lane passed 3/3 on
> 6fc090cf and explicitly proved re-gating plus blocked normal commands after
> trust loss. The reopened reset failure was not reproduced. Its cause remains
> unclassified; these passes are not evidence that the defect is fixed.

### #1634 — Tuwunel aggregate

Durations: 128, 13, 102 s; exits: 1, 1, 1. All fail before `crawl_throttle`:

1. `thread_chip_dot failed: chip dot after threaded read: timed out waiting for the root's thread unread`.
2. `unnamed: unexpected state (join_rule=Invite, has_name=false)`.
3. `thread_summary failed: Thread panel and Room summary did not converge`;
   `thread_panel=true room_summary=false projected_root_activity=false
   root_summary=false reply_count=none expected_count=1`.

Focused `search_crawler`: 70, 71, 70 s, all exit 0 with
`crawl_backfill=ok`, `crawl_throttle=ok`, and `crawl_failure=ok`.

The old catch-up race and its #1699 correction are documented in the issue;
baseline includes `e2c2632e`. The new failures belong to thread read/projection
and create-room settings hydration, not that old oracle. The second failure is
emitted by `scenarios/rooms/unnamed_public.rs` immediately after loading the
created room's settings. A transient SDK projection and an actually wrong
server state have not been distinguished. No expectation was weakened.

> Draft for #1634: Three Tuwunel aggregate attempts on 6fc090cf all stopped
> before crawl_throttle, at thread-chip clearing, unnamed-public-room join
> rule, and thread-summary convergence respectively. The focused crawler lane
> passed 3/3. This does not prove the aggregate crawler stage healthy or
> reproduce its original failure. The new blockers are preserved separately.

### #1636 — Synapse aggregate

Durations: 131, 132, 151 s; exits: 1, 1, 0.
All three runs have `reader_scope_live=ok`, `read_receipt=ok`, and
`live_signals=ok`. The original reader failure is 0/3; the aggregate is 1/3.
Attempts 1 and 2 stop later:

```text
crawl_throttle: speed change started 1 crawl(s) over (catch_ups=1)
crawl_throttle: speed change started 2 crawl(s) over (catch_ups=1)
```

These are not the original `Some(Queued)` oracle. Baseline contains
`c2704cad` / #1697 and `4eb8c31e` / #1698 for the original reader/leave faults.

Current crawler failure ownership: `scenarios/search.rs` counts `initial`
enqueues from the **process-wide** diagnostic ring since an epoch-millisecond
boundary. `crates/koushi-core/src/search.rs::handle_rooms_available` emits
`initial` when a room is not yet known to that actor. The token contains a cause
but no scope identifying the target room or account. Consequently an initial
crawl of a newly discovered room/another account, an enqueue in the boundary's
same millisecond, and an actual unwanted restart cannot be distinguished from
these logs. A global-count false positive is a hypothesis, not an established
root cause. Parked: add a privacy-safe, correlated observation in a separate
change if needed, rather than suppressing the error or changing product policy.

> Draft for #1636: The original reader stage passed in all three Synapse
> aggregate attempts on 6fc090cf. One aggregate passed fully; two failed later
> at crawl_throttle with initial-enqueue counts 1 and 2, each with one catch-up.
> The current unscoped diagnostic count does not identify an actual target-room
> restart. These are new crawler findings, not a reproduced reader failure.

### #1637 — history recovery window

Durations: 27, 7, 22 s; exits: 1, 0, 1.
All three contain `history_window_deferred=ok`, `history_window_29_days=ok`,
and `history_window_30_days_expired=ok`. Therefore all three passed the pending
record/backdating point that originally failed. Only attempt 2 completes all
required explicit Core recovery, guard and cleanup tokens.

Attempts 1 and 3 fail with `history window: logout timed out`.
`scenarios/history_key_recovery_window.rs::Participant::cleanup` stops sync,
then bounds `koushi_sdk::logout` to 10 seconds, then attempts store close.
`crates/koushi-sdk/src/client_session.rs::logout` delegates to SDK logout.
This locates the new failure at the SDK cleanup boundary, but does not prove
whether its delay is HTTP/server, SDK background work, or fixture lifetime.
It is not the missing acceptance record. Increasing the timeout without a
causal trace would mask the problem, so cleanup is left unchanged.

> Draft for #1637: All three local Tuwunel runs on 6fc090cf passed the deferred,
> 29-day and 30-day proofs, so the original missing-pending-record signature
> did not recur. The complete scenario passed 1/3; two attempts failed at SDK
> logout cleanup's 10-second bound. This new cleanup failure is unresolved and
> must not be reported as a fully green history-recovery lane.

### #1279 — user verification

Durations: 99, 101, 69 s; exits: 0, 0, 0.
All three contain `user_verification_request_sent=ok`,
`user_verification_incoming_request=ok`, `user_verification_sas_match=ok`,
`user_verification_identity_verified=ok`, and `user_verification=ok`.

The issue's prior receiver diagnostics (`b_flow=idle`, target not matched,
sync running) locate the gap before a receiver product flow exists, but do not
isolate server delivery, SDK processing, or Core publication. The existing
`crates/koushi-sdk/src/e2ee.rs::observe_incoming_verification_requests` path
and `scenarios/user_verification.rs` waiter remain the next investigation
points. No deterministic receiver-path failure was established here.

> Draft for #1279: The disposable Tuwunel user_verification scenario passed
> 3/3 on 6fc090cf, including incoming request, SAS and identity verification.
> The B-idle timeout was not reproduced. As earlier issue evidence already
> notes, intermittent local passes are not a fix; receiver-path ownership is
> still unresolved. No product or target-matching rule was changed.

## Contract gaps noticed, not changed

These are separate follow-up items, not explanations proven by this experiment:

- `AGENTS.md` requires `KACHE_EXPLAIN_MISS=1` on Rust build invocations, but
  `scripts/lib/local-homeserver-qa.mjs::minimalEnvironment` strips that variable
  from the runner's Cargo children. Explicit prebuilds used the required flag;
  setting it on the runner does not preserve dependency-digest diagnostics for
  a child rebuild. kache itself remains active through Cargo's wrapper config.
- The QA-lane contract requires one absolute deadline across unrelated events.
  `scenarios/identity.rs::wait_for_identity_reset_done` instead recreates
  `timeout(E2EE_EVENT_TIMEOUT, recv_event())` inside its loop. No starvation
  occurred here; this is not evidence for #1612's reset failure. Consolidating
  it with the existing deadline path should be its own bounded change.

## Handoff and verification

Only this report is changed. No product code, SDK pin, runner contract, or
user guide behavior changed; no new behavior test or design review is claimed.
The report is one documentation deliverable, not a PR mixing fixes to seven
issues. If a later root-cause investigation produces fixes, each issue gets its
own finished commit/PR as requested. None of the draft comments above was posted.

Pre-PR checks: SDK gitlink guard, agent-doc checker, affected local Markdown
links/source and command references, secret scan, and `git diff --check`.
The 25 QA attempts and explicit token readback above are the runtime evidence.
Workspace tests, clippy, frontend and GUI suites were not run: no executable
code, dependencies or QA contracts changed. No CI rerun or merge is part of this
report. Outstanding: classify #1612/#1279, isolate the new aggregate/cleanup
failures, and obtain a Tuwunel aggregate run that actually reaches its crawler.
