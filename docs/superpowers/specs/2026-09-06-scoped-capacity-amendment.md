# Proposed scoped capacity/failure amendment

Status: **Correct-to-adopt**, reviewed by Sol (GPT-5.6, high, read-only), then
adopted into scoped-view-lifecycle.md before implementation. No material adoption
findings remained. This does not approve host enablement.

The normal keep-last-installed rule remains: coalesced window/source/profile
updates retain the installed model until a replacement is ready. This proposal
adds an explicit exception for failure to admit required work within the published
hard budgets, counter exhaustion, or unexpected producer failure.

- Initial subscribe failure returns its typed error and releases its provisional
  scope; it cannot wait indefinitely for budget.
- A live scope that cannot reserve the required bounded raw/builder/model work
  retires with Capacity. Counter exhaustion and unexpected producer exit retire
  with distinct CounterExhausted and ProducerFailed reasons, respectively.
- Terminal control bypasses model ACK. The owning host clears the derived view,
  releases resources and announces the localized reason. It does not display a
  partial/empty replacement, silently show stale data as current, or auto-reopen.
- Reopening is an explicit new request. Nothing is evicted from durable product
  state, no pending user mutation is discarded, and unrelated scopes are not
  retired. Charges held by a previously delivered model remain until actual Drop.
- Normal source/dependency updates that fit budgets continue to preserve the old
  installed model. This is not permission to close on ordinary coalescing, stale
  draft rejection, temporary SDK work, or a missed wake.

Rationale: a bounded derived subscription cannot guarantee progress while other
retained owners permanently occupy its budget. A typed terminal result is smaller
and more observable than adding a paused/error control protocol or indefinite
budget-retry scheduler. The conservative initial 64-MiB preparation reservation
can reject work even below the aggregate actual-data ceiling; this limitation must
be documented and exercised in admission tests, not called a heap limit.

The lifecycle keep-last-installed rule has been amended in place. Add the closed
retirement reasons plus localization/host behavior before enabling this consumer.
The routing capacity contradiction is resolved; host enablement remains blocked
on the other explicitly outstanding integration and verification obligations. No HTTP, performance, portability or broader
issue requirement is waived by this amendment.
