# Independent Design Review

Date: 2026-06-11
Reviewer: independent sub-agent
Scope: design review only; no files edited by reviewer

## Reviewed Repositories

```text
matrix-desktop       67159ee Add upstream reference license policy
matrix-rust-sdk      72d2157 chore(deps): bump crate-ci/typos from 1.46.3 to 1.47.0
element-x-ios        8262533 Update GitHub Actions to v6.0.3
element-x-android    99b7c758 Merge pull request #6954 from element-hq/fix/try-fixing-flaky-location-timeline-item-screenshots
seshat               33279d2 Merge remote-tracking branch 'upstream/main' into bump/v4.1.0
element-web          eb6f973284 Merge remote-tracking branch 'upstream/develop' into shinaoka/tokenizer-mode
fluffychat           f5c87bb Merge pull request #3085 from krille-chan/krille/display-tofu-state-in-encryption-page
matrix-dart-sdk      280bc34 Merge pull request #2364 from famedly/krille/increase-timeout-for-create-key
```

## Findings

### Critical

1. Search is not implementation-ready as originally specified.
   `matrix-sdk-search` currently lacks tokenizer configuration, persisted tokenizer/schema metadata, robust rebuild behavior, late-decryption indexing coverage, and UI-ready snippets/highlights.
2. The Slack-like Spaces/DM sidebar cannot be assumed to fall directly out of `matrix-sdk-ui`.
   The desktop app needs its own composition layer over SDK streams, with explicit DM, nested Space, multi-parent room, and unread aggregation rules.

### Important

1. Thread support must be conditional.
   MVP should start with focused thread timelines and disable subscriptions unless proven.
2. Key management needs concrete API choices.
   Implementation planning must decide passphrase vs raw-key SDK paths, zeroization, credential-store naming, missing-secret recovery, and deletion behavior.
3. Element X mobile license provenance must be enforced before direct code ports.
4. Desktop platform requirements were under-specified.
   Signing, notarization, single-instance behavior, store locking, OIDC deep links, notifications, logs, proxy/cert behavior, and blocking command paths must be covered.

### Minor

1. Search design should name `experimental-search` and the exact SDK patch location.
2. DM classification should be pinned and used consistently.

## Outcome

The review recommended no-go for a full app implementation plan until prerequisite spikes prove:

1. `matrix-sdk-search` capability patch;
2. desktop sidebar composition over SDK UI services;
3. concrete key and credential-store integration.

The design spec was updated to reflect these blockers before implementation planning.

