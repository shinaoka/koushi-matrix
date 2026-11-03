# #1770 — the media cache entry names what was downloaded

## Decision

`download_media_for` (`crates/koushi-core/src/timeline/media.rs`) built its
cache path from the room and event hashes alone and returned a cache hit before
it looked at the requested `MediaDownloadSelection`, so whichever selection
wrote `<event-hash>.bin` first answered every later request. The GUI writes
`File`; the MCP `get_attachment` tool writes `Thumbnail { 800, 800 }`, so the
800 px JPEG was served as the original to the timeline, the media viewer and
Save until the cache was cleared.

Read before editing: `REPOSITORY_RULES.md` (canon-first, root-cause discipline,
test placement), the media download contract in
`docs/architecture/state-machine.md`, the #1619 worklog that introduced the
opaque ref, and `crates/koushi-core/src/media_download_ref.rs`.

Canon amended in the same change: the contract line in
`docs/architecture/state-machine.md` now names both ref forms instead of one.
`File` behaviour, the ref prefix, and the cache directory layout are unchanged.

Shape:

- `download/<room-hash>/<event-hash>` — the original. Unchanged string, so
  existing refs, cache entries and adapters keep working with no migration.
- `download/<room-hash>/<event-hash>/<width>x<height>` — a thumbnail of that
  event, resolving to `<event-hash>-<width>x<height>.bin` beside the original.
  The requested size is part of the key, so a second size keeps its own entry
  instead of reintroducing the same collision at a smaller scale.
- Every component is still validated as a short decimal run — the size as two
  of them joined by `x` — and extra parts are still refused, so the grammar
  cannot leave the managed cache root. `download/1/2/3` is still rejected.
- The writer derives its path through `media_download_cache_path`, the same
  resolver the platform adapter uses, so a written entry is always one an
  adapter can find again; no second filename rule exists.
- A thumbnail deliberately does not reuse an existing `File` entry (commented
  at the cache hit): the original is not the requested size.

Out of scope: #1769 (reply-quote thumbnails). The `koushi-media` protocol URL
builder stays `File`-only because no GUI path requests a thumbnail download;
the save flow resolves a thumbnail ref through the same core resolver.

## Verification

- RED before the fix: the new regression test failed on
  `download/1228832746460273/2024224564438182` for both selections.
- GREEN: `cargo test -p koushi-core media` — 60 passed, including
  `timeline::media::tests::a_thumbnail_download_and_a_file_download_of_one_event_stay_separate`
  (both orders, refs, paths and bytes) and the three
  `media_download_ref::tests` cases.
- `cargo test -p koushi-desktop media_download` — 4 passed, including the
  thumbnail-ref resolution of the save path.
- `cargo test -p koushi-core-testkit --test contracts` — 186 passed.
- `cargo fmt --check`; `cargo clippy -p koushi-core -p koushi-desktop
  --all-targets -- -D warnings`; `node scripts/check-leaf-crate-boundaries.mjs`;
  `node scripts/check-diagnostic-test-isolation.mjs`;
  `node scripts/check-rust-test-structure.mjs`; `node scripts/check-agents-docs.mjs`.
- No user-guide change: no menu path, label or documented outcome moved.

## Known limits

- The in-flight gate in `handle_download_media` is still keyed by event id, so
  two selections requested for one event at the same time answer the second with
  `already_in_progress`. The two entries no longer collide; the gate is a
  separate concern and is left unchanged here.
- An entry a pre-fix thumbnail already overwrote as `<event-hash>.bin` keeps
  serving that JPEG for `File` requests until it is cleared. Invalidating it
  would change the `File` name and lose every existing cache hit, which the fix
  deliberately avoids; the new comment in `download_media_for` states it.
- The size component keys the requested selection, not the stored pixels: an
  event carrying its own thumbnail source serves those bytes for any requested
  size, and encrypted media without one downloads the original regardless of
  format (unchanged SDK/source-selection behaviour).
