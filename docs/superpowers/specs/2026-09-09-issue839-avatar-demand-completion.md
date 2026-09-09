# #839: complete shared avatar-demand ownership

Status: proposed canon amendment; approval required before implementation.
Scope: the remaining #839 contract, not #859 virtualization replacement.

## Source-grounded starting point

- `view_scope_lifecycle.rs` already authorizes connection-owned scopes and retires
  them; `view_budget.rs` caps them at 64. Reader windows already cap capacity at
  256 (`koushi-protocol/src/view.rs`). Reuse these identities and budgets.
- `account/profile.rs` already has the shared cache, six active downloads, 256
  queued resources, per-consumer waiters and bounded retry. The same-session
  canceled-task replacement race is fixed in cd18689. Do not add a second downloader.
- `timeline/receipt_endpoints.rs` retains reader resource identities and leases;
  `view_scope_lifecycle/profiles.rs` tracks profile/thumbnail invalidations. These
  currently do not amount to a common visible-demand owner for every surface.
- `App.tsx` still owns URI reference counts, pending request promises, and the
  own-avatar planner. These are retirement targets, not foundations to extend.

## Upstream comparison

Inspected Element Web 48e4bce28e46b0161dbc8ca6b9dd2a3c2867d0d6:
`apps/web/src/components/views/avatars/BaseAvatar.tsx` selects URL candidates,
advances on image error, resets on reconnection and renders Compound Avatar.
It does not provide Koushi's requested toolkit-independent demand contract.
The previously recorded four-readers / three-plus-overflow compact policy remains
Koushi's policy; do not change it during this migration.

Inspected Element X iOS b6943a6142b217cee3578ee91d1a8173e0fbadce:
`ElementX/Sources/Other/SwiftUI/Views/LoadableAvatarImage.swift` delegates avatar
loading to LoadableImage. `LoadableImage.swift:103–173,246–303` checks cached
content, starts loading with the view task, and cancels unfinished work on
 disappearance. URL changes reset view identity. Koushi preserves cache-first
loading and lifetime cancellation but moves demand/priority/retry ownership into
Rust instead of copying SwiftUI or React ownership. No upstream code is copied.

## Proposed contract

1. **One portable scope lifecycle.** Add avatar-surface subscriptions to the
   existing Core scope API, owned by its connection and admitted against the
   current account/session. Reuse opaque scope IDs, revision serialization,
   capacity errors, account retirement and disconnect cleanup. Never reuse a
   retired scope to accept late observations.
2. **Stable observations, not URI requests.** Each surface reports monotonically
   ordered visible identities or a window against its installed source revision.
   Timeline sources use their existing timeline source identity and item IDs;
   member/reader sources use room/user identities; icon lists use their Rust room,
   Space or invite identities. Rust resolves the avatar. The renderer cannot
   choose a different account, MXC, retry policy or fetching eligibility.
3. **Explicit bounds.** At most 256 visible row identities and eight prefetch
   identities per admitted scope. Oversized/stale input is rejected, not silently
   truncated. Own-profile demand is one identity. Compact receipts retain their
   existing cap and full-reader totals. A 1,500-row list does not imply 1,500
   demands. Reuse the 64-scope budget rather than creating an additional pool.
4. **Serializable Rust demand state.** Put the resolved per-scope visible/prefetch
   demand and its account/session/generation context in koushi-state. Core's
   source resolution updates that state; no Matrix SDK types enter the state
   crate. Profile/source changes invalidate affected observations through the
   existing dependency mechanism. Missing profile data remains a placeholder;
   resolving visible demand must not trigger bulk member-image acquisition.
5. **Durable scheduler handoff.** AppActor publishes the latest bounded resolved
   demand to AccountActor over a dedicated latest-wins watch channel. Closing a
   scope removes its demand from this authoritative value; cancellation must not
   depend on a best-effort queue send. AccountActor reconciles shared resources,
   preserving a download while any live scope needs it. Visible resources precede
   prefetch. Existing six-active/256-queued bounds remain. Excess demand is
   represented as capacity/deferred demand, never an unbounded work queue or a
   lost request that requires React retries. Completion makes deferred demand
   eligible. Session and current-task identity gates remain in force.
6. **Keep rendering contracts.** Reuse portable AvatarThumbnailState/source_ref
   and existing scoped reader resource leases. Tauri creates native display
   handles, not product demand. Do not introduce another generic UI model layer
   merely to carry avatars. Cached bytes remain reusable after demand release.
7. **All required surfaces migrate together.** Message senders, compact receipts,
   full readers, People, Space members, room/Space/invite icons and own profile
   use this contract. Delete App's URI reference-count/promise/planner machinery
   and retire renderer download/cancel-by-MXC APIs at the migration boundary.
   People images use only visible demand; never bulk-fetch member profiles.
8. **Unambiguous diagnostics.** Distinguish visible and prefetch identities,
   distinct deferred/queued/in-flight resources, cache hits, cancellation and
   stale results. Never count unrequested timeline placeholders as downloads.
   Diagnostic records contain counts/closed tokens, not identities or URLs.

## Acceptance before GUI migration

- State/actor tests: stale source/session/observation rejection; close/reopen;
  visible priority; shared-resource retention; cache reuse; failed retry budget;
  queued/in-flight cancellation; late completion; capacity/deferred progress.
- Extend disposable Tuwunel/Synapse QA with 1,500 synthetic members/readers and a
  small observed window. Record actual media request counts, not just DOM counts.
  Verify scroll, close, reopen, sharing, caching and account retirement. Require
  tokens in the runner as well as documenting them.
- Only then wire all GUI surfaces and delete superseded owners. Focused browser
  tests verify observations/lifetimes, keyboard full-reader access, compact bounds,
  English/Japanese text and no changes to IME ownership.
- Run the applicable full gates and coherent self-review before the existing
  single deliverable is proposed as one PR. No #839 completion claim at this stage.

## Approval boundary

This changes the cross-actor/public observation contract. Approval authorizes the
canon amendment and Phase A implementation, not bypassing any headless evidence,
GUI migration, CI, human PR approval or the goal's final audit.
