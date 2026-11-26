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
- One policy per concern: the timeline, not a card, decides what fetches itself.

## Change

1. `components/timeline/filePreview.ts` — `filePreviewKind` maps the category to
   `text | audio | video | none`, `canAutoFetchPreview` decides the one automatic
   fetch, and `textPreviewLines` renders the card's few lines. The 256 KB cap
   lives here (`TEXT_PREVIEW_MAX_BYTES`) and is mirrored by the backend's own
   `MAX_TEXT_PREVIEW_BYTES`; the backend value is the enforced one.
2. `TimelineView.tsx` — the existing per-event preview fetch loop now covers "a
   rendered image, or a text file the card can preview", so there is still one
   latch (`requestedPreviewEventIdsRef`) and one failure release. Audio and video
   are excluded on purpose.
3. `TimelineMedia.tsx` — a text card reads its downloaded bytes once and shows
   `TEXT_PREVIEW_LINE_COUNT` lines; the lines are a `<button>`, so the keyboard
   reaches the whole file, and selecting the card runs the same action. An audio
   or video card mounts `<audio controls>` / `<video controls>` over
   `mediaSourceUrl(source_ref, mime_type)`; its background click stops saving
   because the player is the target, and its tile's not-ready click only fetches.
4. `backend/linkMediaPort.ts` — `readDownloadedText`, and
   `TimelineTransport.readMediaText` carries the row's account tab the way
   `saveMediaFile` does, so a read that lands after an account switch still
   resolves that row's cache. The Tauri port calls the new
   `read_downloaded_media_text` command, which resolves the file by tab inside the
   account's managed cache, runs the read off the async worker, and refuses
   anything above the cap; the browser shell port rejects, which leaves the plain
   card.
5. `media_download.rs` — `read_managed_text`, the audio MIME tokens the player
   needs, and byte-range answers (`206`, `Content-Range`, `Accept-Ranges`) because
   a media element seeks and AVFoundation/WebView2 refuse a range-less handler.
6. `tauri.conf.json` — `media-src koushi-media: http://koushi-media.localhost
   https://koushi-media.localhost` on `csp` and `devCsp` (without it
   `default-src 'none'` blocks every media element, including the media viewer's
   existing `<video>`). The release preflight requires both the scheme and the
   Windows origin.

## Why a command instead of a URL

Serving text through the protocol would need a new token plus a `connect-src`
addition for `fetch()`, and `fetch()` on a custom scheme is not something this
app has ever exercised on WKWebView. A bounded string the adapter reads in
process needs neither: no bytes are re-encoded as a URL, the cap is enforced
where the file lives, and the renderer still holds no path. It adds no state, no
Core change and no DTO field.

## Deviations recorded

- The card's lines are a plain `<pre>`, not the message code block: that renderer
  brings HTML sanitizing, the wrap preference and a copy button the card does not
  want here. The height follows `TEXT_PREVIEW_LINE_COUNT` through a CSS custom
  property rather than a second literal.
- Reads are per card mount, not cached per attachment: scrolling a room with
  several small text files out of and back into the window re-reads them from the
  local cache. Bounded by the cap; accepted rather than adding a text cache with
  its own eviction policy.

## Verification

- RED before the change: reverting only `TimelineMedia.tsx` fails 6 card tests
  and 3 of the 4 new e2e cases (`download_media` 0 vs 1, no `<audio>`/`<video>`);
  reverting the widened `TimelineView` filter fails
  `auto-fetches only the text attachment a card can preview`. The above-limit
  cases pass on both, which is their point.
- `cargo test -p koushi-desktop --profile ci`: 362 passed.
- `cargo fmt --check`, `cargo clippy -p koushi-desktop --all-targets`: clean.
- `npm run typecheck`, `npm run lint`: clean.
- `npm test -- --run`: 219 files / 2772 tests.
- `npx playwright test e2e/media-attachments.spec.ts e2e/timeline-*.spec.ts
  --project=chromium --project=webkit --repeat-each=2`: 336 passed, 4 skipped.
  Budget constants were not touched.
- `node scripts/user-help.mjs --check`, `node --test scripts/user-help.test.mjs`,
  the structural `check-*.mjs` gates: ok.

## Not checked

- The real Tauri webview: the harness stubs `invoke`, so `<audio>`/`<video>` are
  asserted by `src`, not by playback. Whether WKWebView and WebKitGTK decode a
  given codec is the engine's decision; an undecodable file shows the browser's
  own error inside the player. The byte-range answer above is what makes playback
  reachable at all, and it is unit-tested, not observed in the app.
- `media-attachments.spec.ts` is in the WebKit opt-in, so the new cases run on
  both engines; an engine that refused `koushi-media:` in a media element would
  still only show up as a player that never loads.

## Stage 2: PDF (#2096)

- `read_downloaded_media_pdf` returns the bytes as a raw IPC response (no JSON
  number array), through the same tab routing as the text reader. Cap
  `MAX_PDF_PREVIEW_BYTES` = 32 MB, refused rather than truncated; a body without
  `%PDF-` in its first KB is refused too.
- `pdfjs-dist` loads lazily (`pdfRenderer.ts`); its worker is a same-origin Vite
  asset. The card draws page 1 offscreen for files within 32 MB and 200 pages;
  the viewer is a modal dialog (like the text one) with paging, page count and zoom.
- CSP: `worker-src 'self' blob:` on `csp` and `devCsp`; the release preflight
  requires it.
- Not verified here: the worker under the real Tauri webview CSP (the browser
  e2e harness does not enforce the CSP). pdf.js fetches CMaps, standard-font data
  and wasm by URL; `connect-src` stays closed, so non-embedded base-14 fonts and
  JPX images fall back or fail to draw.
