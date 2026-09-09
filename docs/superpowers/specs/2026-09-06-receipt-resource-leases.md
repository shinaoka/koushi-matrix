# Receipt resource leases — implementation proposal

Status: private 32+96-MiB byte-lease layer approved Correct-to-implement by Sol
(GPT-5.6, high, read-only). Acquisition/capture must be synchronized, arithmetic
must reject oversubscription, and uncharged entry Arcs must not escape. This is
part of the approved reader vertical, not approval of public avatar/UI delivery.

## Concrete problem and reuse

The current renderable_thumbnail cache owns Vec bytes in a 256-entry/32-MiB LRU.
A Ready DTO contains an opaque avatar/hash reference. Eviction removes its bytes;
a retained Ready model alone cannot guarantee that reference remains readable.
Reuse that cache and portable reference format. Do not create another image cache,
profile database, downloader or renderer-owned demand policy.

## Minimal byte ownership

- Store each existing cache entry behind Arc. Insertion still moves its input Vec;
  ordinary lookup still produces the existing owned transport bytes with one copy.
- A private lease retains the immutable entry, so LRU eviction does not invalidate
  its bytes. Cloned handles share one lease charge. No filesystem/SDK calls occur
  during lease acquisition or release.
- Keep the existing LRU limits unchanged. Cap additional lease charges at 96 MiB,
  so LRU bytes plus lease-retained bytes are conservatively bounded by 128 MiB.
  Charge each independently acquired lease, even if its entry is shared; this may
  overcount shared bytes but cannot undercount retention. No heap-size claim.
- A lease charge is RAII-owned until its final clone drops, including after cache
  clear. Distinguish missing/invalid source and capacity failure. Never advertise
  that a lease was acquired when it was not.
- Cache clear removes discoverability immediately. Held immutable bytes may exist
  until final delivery/native owners release them; access after scope retirement
  must still be rejected by the common scope authorization, not by the cache key.

## Actual integration required in this vertical

AppActor must acquire resource bindings before discarding raw MXCs or publishing
ReaderReady. Prepared/in-flight and installed model metadata retain the required
lease set, sharing handles rather than byte copies. Retirement/source change/Drop
release their handles. Retained client deliveries remain charged until released.
A missing/capacity-limited resource must be handled by the Rust demand policy
before final publication, not silently repaired by the serializer or UI.

The adapter resource read must validate its authenticated consumer, live scope
and installed resource mapping before reading lease content. Bare cache hashes
are not new authority. Unready avatars still need private MXC bindings and the
shared bounded demand/queue/attempt-generation lifecycle. This proposal does not
waive that work or authorize exposing the existing private reader getter to hosts.

## Verification before enabling public delivery

Prove retained bytes survive LRU eviction but do not become globally discoverable
again; final-handle release returns the charge; clone shares one charge; failed
admission preserves existing charges; aggregate charged retention remains bounded.
Then verify actual model→ACK→installed/retirement ownership and unauthorized read
rejection through the Core/host boundary. Preserve existing cache, offline SDK,
profile, avatar and link-preview tests. HTTP cancellation/dedup/demand remains its
own required actual-network evidence, not proven by byte leases.

## Review question

Is this conservative 32+96-MiB ownership approach the smallest safe extension of
the existing cache, or should the shared 128-MiB live-byte charge instead be owned
once per entry (including its LRU and all leases)? Review before changing code.
