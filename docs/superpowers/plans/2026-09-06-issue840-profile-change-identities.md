# #840: retain display-label identities at profile mutation

Status: implemented and reviewed; required CI pending.
This is the mutation-reporting part of the common publication migration, not
completion of #839/#840. It does not fix the pending 1,500-reader profile-lookup
regression; that belongs to compact/full-reader demand cutover.

## Source evidence before implementation

- `reducer/profile.rs` knows the own user, incoming profile IDs, and alias target
  but reduces those facts to bare `UiEvent::ProfileChanged`.
- `runtime.rs::handle_ui_event_effect` consequently calls one global label
  builder. A separate command-side `additional_user_ids` parameter exists just
  to preserve an alias-cleared user after removal from the alias map.
- `event_projection.rs::derive_display_label_updates_for_user_ids` is not
  scoped: it enumerates every alias and cached user plus the own user before
  its supplied IDs. Status/avatar-only changes also trigger that enumeration.

## Minimal change

Refine the existing `UiEvent::ProfileChanged` payload; do not introduce another
bus, registry or action-to-change classifier. Add one small pure Rust record
`ProfileDisplayChange { user_ids: Vec<String> }`, with Default for no label
change and private-safe Debug reporting only the count. It carries label
identities, not a claim to enumerate every profile metadata mutation.

The coarse profile notification continues to exist for state/operation changes;
its label payload says which identities need label re-resolution:

| Mutator | Display identities |
| --- | --- |
| Own profile received or successful own profile update | Current own user, if present |
| Incoming user-profile batch | IDs of the incoming batch, retained before consumption; deduplicate at emission |
| Alias update requested, including removal | Exactly the alias target, retained before map mutation |
| Persisted aliases replaced | Union of previous and newly admitted alias keys, including removed aliases; this is an explicit load/replacement path |
| Alias/profile operation status or failure without label rollback | Empty |
| Ignore-list changes and their status/rollback | Empty: filtering changes remain their existing Rust effects, not label changes |
| Thumbnail status updates | Empty: image status is not a display-name change |
| Session/profile reset | Empty; existing session/timeline reset and fresh projection remain authoritative; verify reset cases rather than adding a global relabel fallback |

Pass this payload through `profile_changed_effects`. Core resolves/emits only
its supplied IDs. Remove the command-side additional-ID parameter and its
special alias extraction once the reducer supplies the same fact. The label
builder must no longer implicitly union cached users/aliases/own identity.
Remove the unused all-known-label wrapper; migrate its existing test's explicit
identity input while retaining every alias/upstream/own/unknown-name assertion.

State transition ordering, alias persistence/rejection behavior, global
StateDelta delivery and command admission stay unchanged. The remaining eager
StateDelta/watch copies, profile/receipt refresh scans and scoped subscriptions
remain explicit migration work; this slice does not claim those bounds.

## Proposed canon amendment

Add to the existing profile/display ownership contract:

> Profile mutations retain display-label identities in their existing Rust
> change notification. Routine label publication resolves only those identities;
> status/image-only notifications do not trigger all-user relabeling. Alias
> removal retains its target identity after deletion. An explicit alias-store
> replacement accounts for both old and new keys. Initial event projection and
> session/timeline reset remain their existing owners.

## Verification

1. Strengthen the existing label-resolution test with 1,500 unrelated cached
   users and a one-user request. Baseline returns unrelated identities; assert
   exactly the requested identity (plus existing label correctness) for RED.
2. Migrate existing reducer tests to the refined payload and add only missing
   assertions for exact alias removal/load identities and empty status payloads.
   Preserve old test items; do not add a per-mutator Cartesian product.
3. Run the existing runtime alias test through real command/action handling to
   verify scoped publication and alias-clear fallback; keep admission, state
   generation, failure and session-reset tests. No renderer fake as proof.
4. Run State and Core tests, Core testkit, doctests for the new public record,
   formatting and applicable boundary/docs checks, then exact-head required CI.
   The public record's Debug must not expose user IDs. No frontend/wire change
   is intended by this internal effect payload; confirm that with callers.
5. Parent full diff self-review, then one independent final review before merge.

Design review: GPT-5.6 Sol, read-only, high — **Correct-to-implement;
canon approved**, no findings. Reviewed every profile mutator, reset, room-local
profile distinction, Core alias workaround and existing resolution/runtime
coverage before implementation.

Full-diff review: DeepSeek V4 Flash, read-only, medium —
**Correct-to-merge subject to required CI**, no material findings. Reviewed
patch SHA-256 `c162223df7cecf277d85da716ec4d91adf34e9d978ca1d2a828effc77a43fedc`
and the referenced test logs. Optional unchanged-batch and explicit alias-load
optimizations are not needed to satisfy this approved contract; no further
review round or speculative change was requested.

## Local evidence

- Existing label-resolution scenario strengthened with 1,500 unrelated cached
  users: RED **1,505 versus expected 1**, then GREEN. Existing alias/upstream/
  own/unknown resolution assertions remain. Runtime alias-clear now also proves
  exactly one emitted label; reducer cases verify removal, replaced alias keys
  and an empty status-completion payload.
- Core: **944 passed, 8 existing ignored**, 5.24s. State: **786 tests plus one
  public-record doctest passed**. Core testkit: **225 passed**. No existing
  behavioral test item was removed.
- Formatting, SDK guard, domain-dependency, Rust-test-structure and docs checks
  passed. No frontend or transport DTO shape changed. Parent reviewed the full
  diff, including the new record's privacy-safe Debug and all notification sites.
- Compile-only setup was separated after 60-second compilation limits. One
  direct test-binary invocation bypassed the repository Cargo-configured
  `RUST_MIN_STACK=4194304` and overflowed its default stack. The exact case passed
  through Cargo; the full Core suite then passed through Cargo. No code or
  checked-in stack/time limit was changed to hide this invocation mistake.
- Evidence logs: `/tmp/issue840-profile-red.log`,
  `/tmp/issue840-profile-green.log`, `/tmp/issue840-profile-core-final.log`,
  `/tmp/issue840-profile-state-final.log`, `/tmp/issue840-profile-testkit.log`.
  The separate receipt-profile-lookup RED remains on the integration worktree,
  excluded from this independently mergeable label-publication change.
