# Issue #866: confirm alias edits once

## Evidence and decision

Both ProfilePanel and TimelineView previously called the alias-save callback
from every input change. A four-step synthetic edit produced four callbacks
before Done. App's latest-text mutation lane waits for command admission, not
SDK persistence, so this is not a debounce of storage work. AccountActor awaits
the SDK account-data update, then publishes aliases and profile changes; this
turns intermediate input into unnecessary saves and display projections.

Retaining autosave would require coalescing around the authoritative save
lifecycle, rather than extending frontend timers or treating admission as
completion. The user explicitly selected Done/Enter confirmation on 2026-09-10.
The existing Done button therefore submits one final value; typing, dismissal,
and target changes do not write. A non-composing Enter submits through
ImeSafeForm, while IME-confirmation Enter remains native composition work.

Rust remains the alias, save/error, reconciliation and display-projection owner.
The existing bounded App mutation lane still sequences confirmed edits. No
optimistic label projection or new frontend save state machine was introduced.
Profile target/context changes and timeline-key changes discard open drafts.

## Verification

The new AliasSubmission component tests failed before implementation and passed
afterwards. Four input changes now produce zero callbacks while editing and one
after Done, for both editor surfaces. Tests also cover Japanese composition,
clear-to-null, dismissal, and profile target switching. The actual frame-gap
maximum reported in the issue was not reproduced; this change removes measured
command amplification, not an independently proven crawler or rendering stall.

The existing browser alias scenario now types character by character, asserts
zero additional commands until confirmation, verifies both Done and Enter, and
checks Rust-projected names and clear behavior. A focused Rust test verifies
matching save failure remains recoverable, authoritative reload restores prior
aliases, and another confirmed edit can start. Backend storage behavior itself
is unchanged.

Validation: all frontend tests (1,322), TypeScript typecheck, desktop lint,
IME inventory/checker tests, and the Rust alias failure/profile targets (31)
passed. Five relevant browser alias/receipt tests and the frontend production
build also passed. Self-review covered the final diff and the
confirmed-submit policy approved by the user.

Contracts: REPOSITORY_RULES.md, engineering-rules.md alias UI ownership,
state-machine.md profile failure/reconciliation, and docs/agents/verification.md.
The engineering policy and current ownership inventory were updated to describe
confirmed submission; historical autosave plans remain historical records.
