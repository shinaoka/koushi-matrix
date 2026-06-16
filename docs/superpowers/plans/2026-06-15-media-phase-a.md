# Media And Files Phase A/B Implementation Plan

Date: 2026-06-15
Status: Phase A Rust/headless and Phase B headless GUI wiring implemented.
Caption follow-up for #34 is implemented with local core and Linux GUI gates
passing on 2026-06-16.

> **For agentic workers:** implement Phase A before any GUI work. Use mini
> agents only for bounded investigation or isolated patches; the main agent
> owns architecture, review, verification, and merge readiness.

**Goal:** add a Rust-owned media/file state-machine contract for timelines:
timeline events carry media metadata, media upload uses core commands/effects
with progress, and headless local QA proves send/receive/download without
putting media logic in React.

**Architecture:** `matrix-desktop-core` owns media commands, SDK effects,
upload progress, download completion, and timeline media projection.
`matrix-desktop-state` remains serializable product state. React may later
render file pickers, progress, bubbles, and download controls, but it must not
infer Matrix media semantics.

## Phase Boundary

- Phase A is Rust/headless only:
  - `TimelineItem.media` DTO and SDK projection.
  - `TimelineCommand::UploadAndSendMedia`.
  - upload progress and media download CoreEvents.
  - media captions as optional `FormattedMessageDraft` on a single media event.
  - private-data-free headless QA tokens: `send_media=ok`, `media_caption=ok`,
    `image_compress=ok`, and `recv_media=ok`.
- Phase B is GUI:
  - attach control, file picker, progress UI, image/file bubbles,
    click-to-open/download, and i18n copy.
  - headless browser fixture proving the GUI stages one attachment, sends the
    Composer draft as the media caption, and does not dispatch a separate
    text-event fallback.

## Rules

- Filenames, captions, event bodies, and bytes are visible/private UI content.
  They may cross the command/event boundary only when needed for product
  behavior and must never appear in Debug output, QA logs, or diagnostics.
- Encrypted media keys and hashes stay inside Rust actor-private caches. Webview
  DTOs may expose only safe metadata: MXC URI, encrypted flag, encryption
  version, mimetype, size, dimensions, and thumbnail metadata.
- CoreEvents never carry downloaded bytes. Download completion reports
  `byte_count` and correlation IDs only.
- Local homeserver QA uses synthetic data and prints only success tokens.
- Multi-attachment semantics are intentionally narrow until the upload UX
  issue owns batching: one staged attachment per Send; selecting another file
  replaces the staged attachment.

## Tasks

### Phase A

- [x] Add RED tests for media DTO serialization, command Debug redaction, and
  headless QA scenario tokens.
- [x] Add core media DTOs and command/event types.
- [x] Project SDK `m.image` and `m.file` events into `TimelineItem.media`.
- [x] Route `UploadAndSendMedia` through the room send queue with caller-supplied
  transaction IDs and upload progress events.
- [x] Keep encrypted `MediaSource` values actor-private for download effects and
  emit byte-count-only completion events.
- [x] Extend headless local QA with a `media` scenario and the documented
  private-data-free tokens.
- [x] Carry optional media captions through the outbound media request and
  assert the `media_caption=ok` local core token.
- [x] Carry the Rust-owned image compression policy/variant contract through the
  outbound media request and assert the `image_compress=ok` local core token.
- [x] Update TypeScript wire types and checked-in `coreEvents.generated.json`.
- [x] Update architecture/QA docs and operational notes with lessons learned.

### Phase B

- [x] Add RED React and Playwright tests for attach control, media metadata
  rendering, upload progress, and download command shape.
- [x] Add thin Tauri `upload_media` / `download_media` commands that submit
  `TimelineCommand` values only; no SDK calls or Matrix media semantics in the
  adapter.
- [x] Add a Composer file input usable by headless Playwright without opening a
  native file dialog.
- [x] Stage one attachment in the Composer so text entered before Send becomes
  the single media event caption instead of a separate `send_text`.
- [x] Render `TimelineItem.media` in `TimelineView` from Rust-owned DTOs only,
  including upload progress keyed by the local transaction id.
- [x] Render media captions from `TimelineItem.body` / `TimelineItem.formatted`
  below the media metadata row.
- [x] Keep media source details private to Rust/DTO contracts: the GUI does not
  render MXC URIs, encrypted media keys/hashes, or downloaded bytes.
- [x] Update headless app harness responses for `upload_media` and
  `download_media`.
- [x] Render the Rust-owned image upload compression setting and use the
  Rust-owned policy from the snapshot for GUI/effect-layer pixel transforms.
- [x] Add ask-mode image compression choice UI, always-mode automatic
  compression, small-image skip behavior, refreshed thumbnail payloads, and
  selected-variant metadata in `upload_media`.
- [x] Add browser-headless coverage for ask/always/small-image paths and a
  Linux virtual-display `local-image-compression` lane for real WebView canvas
  transform evidence.

## Verification

Focused checks before committing:

```bash
cargo test -p matrix-desktop-core --lib
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml core_event_wire_format_matches_checked_in_contract_artifact
npm --prefix apps/desktop test -- src/App.test.tsx src/domain/timelineStore.test.ts
npm --prefix apps/desktop run test:ui-headless -- e2e/basic-operations.spec.ts --grep "attach control"
npm --prefix apps/desktop run typecheck
```

Behavioral gate when local homeserver binaries are available:

```bash
npm --prefix apps/desktop run qa:headless-local -- --server=both --scenario=media
```

2026-06-16 local verification note: Conduit/Tuwunel tools are available in this
environment. The focused caption follow-up passed with:

```bash
PATH=/tmp/matrix-desktop-local-qa-bin:$PATH \
  npm --prefix apps/desktop run qa:headless-local -- \
  --server=conduit --scenario=media --core --core-backend=both --timeout-ms=240000

PATH=/tmp/matrix-desktop-local-qa-bin:$PATH \
  npm --prefix apps/desktop run qa:linux-gui -- \
  --scenario=local-media --server=conduit --skip-build \
  --artifact-dir=artifacts/linux-gui-local-media-fast --timeout-ms=180000

PATH=/tmp/matrix-desktop-local-qa-bin:$PATH \
  npm --prefix apps/desktop run qa:linux-gui -- \
  --scenario=local-image-compression --server=conduit --skip-build \
  --artifact-dir=artifacts/linux-gui-local-image-compression-fast --timeout-ms=180000
```
