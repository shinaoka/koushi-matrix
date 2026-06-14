# Media And Files Phase A Implementation Plan

Date: 2026-06-15
Status: Phase A implemented; local homeserver media gate pending in this
environment because Conduit is not installed.

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
  - private-data-free headless QA tokens: `send_media=ok` and `recv_media=ok`.
- Phase B is GUI:
  - attach control, file picker, progress UI, image/file bubbles,
    click-to-open/download, and i18n copy.
  - headless browser fixture proving the GUI invokes the Rust command.

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

## Tasks

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
- [x] Update TypeScript wire types and checked-in `coreEvents.generated.json`.
- [x] Update architecture/QA docs and operational notes with lessons learned.

## Verification

Focused checks before committing:

```bash
cargo test -p matrix-desktop-core --lib
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml core_event_wire_format_matches_checked_in_contract_artifact
npm --prefix apps/desktop run typecheck
```

Behavioral gate when local homeserver binaries are available:

```bash
npm --prefix apps/desktop run qa:headless-local -- --server=both --scenario=media
```

2026-06-15 local verification note: `node scripts/desktop-headless-local-qa.mjs
--check-tools` fails in this environment with `conduit is not installed or not
runnable with --version`, so the live local homeserver media scenario remains
pending until the homeserver binaries are available.
