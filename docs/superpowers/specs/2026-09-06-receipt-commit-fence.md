# Receipt source commit fence

Status: Correct-to-implement — Sol (GPT-5.6, high, read-only) reviewed the
proposal and current index/endpoint/registry code. Mandatory condition: split a
prepared-only synchronous commit; never put model::prepare under the source cell.
Public delivery and the other admission gates remain unapproved for exposure.

## Existing gap

ReceiptReaderIndex owns Arc<()> window_epoch. Changed receipts replace it;
Raw/ResolvedReceiptWindow retain Weak<()> and check strong_count. AppActor and
CoreConnection also acquire the existing actor-generation lease. This detects
already-invalid preparation, but checking an epoch and publishing into the scope
mailbox are not atomic against a receipt change or event removal.

## Minimal proposed change

Replace the per-index unit epoch with a private validity cell. The implemented
Arc<Mutex<ReceiptEpoch>> stores validity plus the checked process-monotonic source
revision required by the reviewed routing design. The index is its only
invalidation owner; preparation retains Weak references.

- Before the first logical mutation in update (including timestamp None→0 or a
  thread-only change), lock the old cell and set false. After updating the index,
  install a fresh true cell. Logically equal roots retain the same cell.
- Index Drop invalidates its cell too, so event removal and replacement cannot
  leave an upgraded old witness valid. Invalidation occurs before externally
  publishing the accepted receipt change.
- A private commit operation upgrades the weak cell, acquires the existing actor
  generation lease, locks the cell, rejects false, and runs only the await-free
  mailbox commit while holding it. The guard is released before returning.
- Do not hold the cell while performing SDK/profile lookup, byte preparation,
  serialization or waiting for ACK. Prepare those first. Revalidate current
  profile/locale, consumer, source and requested-window revisions in the AppActor
  commit turn before calling the source fence. These other gates are still needed.
- Lock order is source cell → registry/control/mailbox. No path holding registry
  locks may acquire the cell. Do not call a cell-checking method recursively from
  the fenced callback. Index invalidation does not acquire registry locks.

The receipt mutation may wait for one short mailbox commit; it must never wait
for a host, SDK request or encoding operation. This is not an additional actor,
queue, cache, ordering index or publication authority.

## Verification

Use controlled threads/channels, not sleeps: hold the commit cell while a receipt
update/removal attempts invalidation, prove commit completes before invalidation;
after invalidation prove the old witness cannot commit even if upgraded earlier.
Preserve no-op sharing and None/zero logical-change tests. Then wire the actual
AppActor prepared-model commit and test stale source/window/dependency rejection.
Record primitive fence evidence separately from that integration and from timing
acceptance. Public subscription delivery remains blocked until all gates exist.

## Review question

Is this sufficient and minimal for atomicity against receipt-index change/removal,
assuming the existing actor-generation lease and the stated lock order? Identify
any deadlock or ownership hole before implementation. Do not approve the unfinished
public facade or treat the primitive as end-to-end completion.
