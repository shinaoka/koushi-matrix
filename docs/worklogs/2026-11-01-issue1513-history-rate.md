# Historical-key responder rate ledger (#1513)

The historical-recovery design specifies a 60-second room/requester rate and
nonterminal rate denials. The responder instead retained rate entries for the
30-day episode lifetime and silently returned `TooLarge` when the ledger was
full. The replay cache and episode lifetime are separate and unchanged.

The responder now prunes rate entries at 60 seconds, preserving fail-closed
clock-rollback handling, and sends the authenticated requester a nonterminal
`RateLimited` refusal if 512 live pairs remain. The requester already handles
that wire reason without terminating its correlation. No help text change is
needed: the existing guide describes Retry and does not promise a rate window.

An independent pre-implementation security review recommended the nonterminal
reason and preserving the existing ingress and replay bounds; the independent
post-implementation security review found no actionable issues. The Rust ledger
test failed before the fix and passed afterwards. The disposable Synapse
history-recovery scenario also failed with the old capacity and old retention
branches independently, then passed with both fixes. Its requests and replies
use production Core and encrypted transport; only the rate timestamps and
synthetic ledger entries are fixtures.
