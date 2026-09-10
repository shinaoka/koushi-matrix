# Receipt hover wire and timestamp regression

The scoped reader popup introduced a wire mismatch: `ReceiptSourceRef` reused
the numeric legacy `RequestId` TypeScript shape, while Rust's
`TimelineViewSource` decodes both request ID fields as decimal strings. Opening
the popup with a committed source therefore failed at the transport boundary.
Popups without that source used compact receipts, whose timestamps were
explicitly discarded. These paths explain why some rooms displayed an error
while others displayed names without dates.

The fix aligns the TypeScript source mirror and timeline source construction
with the existing Rust contract, preserves available compact receipt timestamps,
and uses a reader-specific localized failure message. No Rust lifecycle,
subscription authorization, or retry policy changes.

Contracts consulted: `REPOSITORY_RULES.md`, `docs/agents/verification.md`,
`docs/agents/state-ownership.md`, `docs/architecture/i18n.md`, and the existing
scoped wire definition in `crates/koushi-protocol/src/view.rs`.

Verification: a new backend integration test renders TimelineView, hovers a
receipt, and exercises the real Tauri client with a strict synthetic IPC stub.
Both the source-backed and compact-only cases failed before the fix and passed
afterwards. The source-backed case checks the decimal-string payload and model
acknowledgement; both cases check displayed dates. Related reader, client,
timeline live-state, avatar, and localization suites pass (84 tests total).
TypeScript typecheck and focused ESLint pass. Self-review checked the complete
diff against the existing wire and presentation contracts. The user then requested local installation. A release-profile macOS app build
completed with the existing Developer ID identity; strict signature verification
passed before and after installation. Build 2686.2 replaced the installed app,
with the previous bundle retained for rollback. Process launch was checked;
live-account receipt hover validation remains with the user.

## Follow-up: popup geometry

The installed-app check exposed a layout regression missed by DOM-only tests:
restored timestamps wrapped inside the old 260px panel, while placement still
reserved 17px per row (less than the 18px avatar). The full-reader list also
permitted wrapping and did not apply the spacing assumed by the height formula.

The popup now uses a pane-bounded 420px width, fixed 20px rows and matching grid
gaps. Name and timestamp are separate columns: long names ellipsize, timestamps
stay on one line, and the close button has reserved space. The overflow-count row
is included in the height calculation. A real headless browser check reproduced
unwanted vertical overflow for both one and two readers before the fix; both
cases pass afterwards. Synthetic screenshots were visually inspected. The eight
focused receipt tests, TypeScript typecheck, and focused ESLint also pass.

The signed release build 2686.3 was installed and relaunched. Strict signature
verification passed, and the replaced 2686.2 bundle was retained for rollback.
