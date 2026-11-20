# Shortcut-parity table catch-up (#1994)

`docs/qa/shortcut-parity.md` said composer formatting was "not implemented yet".
#1949 (PR #1955) had wired Cmd/Ctrl+B and Cmd/Ctrl+I, and the source, the tests, and
`docs/help/messaging.md` all said so; only this QA record did not.

## What changed

- **Bold and Italic left the deviations table** and became implemented rows, in the
  registry's own order, each naming the behaviour its test pins.
- The rest of that row is now **Composer link and code**: `formatLink` and
  `formatCode` keep registry rows and keys but no handler maps them
  (`inlineFormatFromKey` handles `b` and `i` only), so those two are formatted from the
  toolbar buttons alone. That is a gap in the shortcut surface, not in the formatting
  feature, and the row says which.
- The document defines its two statuses and scopes itself: it records the rows a QA
  pass has verified, and the registry — not this page — is the complete list. It also
  separates its own words from the registry's, whose `implemented: false` means "no
  app-level action for native-menu dispatch" and whose `parity: "deferred"` is what
  App Settings → Keyboard actually renders. It carries a last-checked commit.

## The sweep found two more false rows, not one

`rg -n "not implemented yet|Deferred" docs/qa` returned this file's rows plus two dated
records (`nightly-triage-2026-11-01.md`, `integration-edge-cases.md`) whose "deferred"
refers to their own pass. But the phrase search cannot see a row that is false without
using either word, and two were:

- **Room list sections** said collapsing was "not wired yet". Every section header is
  a toggle button (`Shell.tsx:1613-1622`); only the `ArrowLeft`/`ArrowRight` keys are
  unwired. The row now says that.
- **Autocomplete** said Escape and arrow navigation were inactive "because the feature
  is not present". The composer's mention list takes Arrow Up/Down and Tab
  (`composer.tsx:736-750`) and Rust resolves Escape to `CloseAutocomplete` before it
  can cancel the reply (`composer_shortcuts.rs:163-171`), with three registry rows of
  its own. The row is gone; the Esc row now says it dismisses the mention list first.

The other two deviation rows were checked and kept: page up/down and first/latest
message are neither in the registry nor handled, unread-room navigation is registry
only, and calls stay deferred with the call feature set.

## Not changed

- The registry's `implemented: false` on `formatBold`/`formatItalics` is correct as
  documented there, so no code changed: this is documentation only.
- One pre-existing drift found while checking citations is left for its own issue:
  `docs/help/messaging.md` tells users to use "Jump to latest message" where the
  control's label is "Jump to bottom, {count} new messages" (`messages.ts:4328`).
  (The review also flagged the page's Help-control wording against "Copy repository
  URL"; the actual label is "Copy GitHub URL", `messages.ts:2353`, so there is no
  drift there — that paragraph was removed from this page as a duplicate fact, not as
  a correction.)
- Rows the registry has but this table omits (zoom, fullscreen, upload, Space panel,
  room selection by key, F6 landmarks) stay omitted: the page is a verified subset,
  and making it a registry mirror is not a QA record's job.
