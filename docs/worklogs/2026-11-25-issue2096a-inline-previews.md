# #2096 stage 1: inline attachment previews

Scope: text/source-code and audio/video attachments preview inside their timeline
card. PDFs are stage 2 (a renderer), and macOS Quick Look is not in scope.

## Canon

- Attachments reach the renderer as a `TimelineItemMedia`. `fileTypeCategory`
  already classifies them, so the card knows the category before it downloads.
- The only renderable media URL is the managed-download protocol
  (`koushi-media:`); a downloaded file's `source_ref` is an opaque
  `download/<room-hash>/<event-hash>` string that never leaves the adapter as a
  path.
- The user's Downloads folder is written only by `save_downloaded_media`, which
  the card's tile and hover controls call.

## Change

1. `components/timeline/filePreview.ts` — `filePreviewKind` maps the category to
   `text | audio | video | none`, `canAutoFetchPreview` decides the one automatic
   fetch, and `textPreviewLines` renders the card's few lines. The 256 KB cap
   lives here (`TEXT_PREVIEW_MAX_BYTES`) and is mirrored by the backend's own
   `MAX_TEXT_PREVIEW_BYTES`; the backend value is the enforced one.
2. `TimelineMedia.tsx` — a text card reads its downloaded bytes once and shows
   `TEXT_PREVIEW_LINE_COUNT` lines; selecting the card opens
   `MediaTextPreviewDialog` with the whole file, and the tile keeps saving. An
   audio or video card mounts `<audio controls>` / `<video controls>` over
   `mediaSourceUrl(source_ref, mime_type)`; its background click stops saving
   because the player is the target.
3. `backend/linkMediaPort.ts` — `readDownloadedText`. The Tauri port calls the new
   `read_downloaded_media_text` command (the file is resolved by tab inside the
   account's managed cache and refused above the cap, so no path and no unbounded
   read cross the boundary); the browser shell port rejects, which leaves the
   plain card.
4. `media_download.rs` — `read_managed_text` plus the audio MIME tokens the
   player needs. `tauri.conf.json` gains `media-src` for the same protocol
   (without it `default-src 'none'` blocks a media element), and the release
   preflight now checks that directive on both `csp` and `devCsp`.

## Why a command instead of a URL

Serving text through the protocol would need a new token plus a `connect-src`
addition for `fetch()`, and `fetch()` on a custom scheme is not something this
app has ever exercised on WKWebView. A bounded string the adapter reads in
process needs neither: no bytes are re-encoded as a URL, the cap is enforced
where the file lives, and the renderer still holds no path. It adds no state, no
Core change and no DTO field.

## Verification

- RED before the change: reverting only `TimelineMedia.tsx` fails 6 of the 18
  card tests and 3 of the 4 new e2e cases (`download_media` 0 vs 1, no
  `<audio>`/`<video>`); the above-limit case passes on both, which is its point.
- `cargo test -p koushi-desktop --profile ci`: 361 passed.
- `cargo fmt --check`, `cargo clippy -p koushi-desktop --all-targets`: clean.
- `npm run typecheck`, `npm run lint`: clean.
- `npm test -- --run`: 219 files / 2767 tests.
- `npx playwright test e2e/media-attachments.spec.ts e2e/timeline-*.spec.ts
  --project=chromium --project=webkit --repeat-each=2`: 336 passed, 4 skipped.
  Budget constants were not touched.
- `node scripts/user-help.mjs --check`, `node --test scripts/user-help.test.mjs`:
  OK.

## Not checked

- The real Tauri webview: the harness stubs `invoke`, so `<audio>`/`<video>` are
  asserted by `src`, not by playback. Whether WKWebView and WebKitGTK decode a
  given codec is the engine's decision; an undecodable file shows the browser's
  own error inside the player.
- `media-attachments.spec.ts` is not in the WebKit allow-list, so the new e2e
  cases run on Chromium only. The assertions are attribute-level, but an engine
  that refuses `koushi-media:` in a media element would not be caught here.
