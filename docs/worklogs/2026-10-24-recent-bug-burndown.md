# 2026-10-24 — Recent bug burndown (#1594 #1596 #1597 #1598 #1601)

## Scope

The five bug reports filed against `main` after the 2026-10-22 burndown, fixed
on one branch (`fix/recent-bug-burndown`) so a single review can check them
together. Four are state/projection behavior, one (#1598) is canon wording.

## Canon consulted

- `REPOSITORY_RULES.md` — reproduce with a failing headless check first, no
  manual GUI evidence, root cause before patch.
- `docs/architecture/state-machine.md`
  - "Keyword rules": the duplicate refusal now names the comparison the
    homeserver itself uses (`matrix_case_key`, full Unicode lowercase folding)
    and keeps width/compatibility variants distinct (#1597).
  - "Local Notification Delivery": replaced the stale sentence that said
    notification text stays English until localization is decided. The Rust
    notification catalog resolves at send/retry time (#1598).
  - Timeline message projection: `formatted.plain_text` stays the concatenation
    of sanitized text nodes; every projected offset addresses that string
    (#1596).
- `docs/architecture/i18n.md` — invite search already promised Rust-owned
  normalization and CJK sort keys for mention candidates; the invite target
  query now follows the same policy (#1601).
- `docs/superpowers/specs/2026-07-19-timeline-scroll-stability-design.md`
  (#277) — an image row's box comes from event metadata and must not change
  when the download completes; #1594 restores that invariant after a width
  change.
- `docs/agents/verification.md` — RED-then-GREEN.

## Change

- **#1597** — `koushi-state::matrix_case_key` mirrors Synapse's push-rule
  matcher (`str::to_lowercase()`): full Unicode lowercase folding, no NFKC. Both
  the state refusal (`state/account_notifications.rs`) and the SDK write plan
  (`koushi-sdk/account_notifications/keyword_rules.rs`) compare that key, so
  `café`/`CAFÉ`, `σίγμα`/`ΣΊΓΜΑ`, and `ДОМ`/`дом` are duplicates while a
  fullwidth `ｃａｋｅ` is not the same rule as `cake`.
- **#1601** — the invite target query normalizes with
  `normalize_cjk_search_text` (NFKC + case fold), matches display name, aliases,
  and Matrix ID on the folded text, and orders candidates with
  `cjk_display_sort_key`, so `会議2` sorts before `会議10` and `エリス` matches
  `ｴﾘｽ`.
- **#1596** — `koushi-core::link_preview` gained
  `extract_formatted_link_ranges`. It detects URLs on the block-separated text
  (the same boundary policy the preview extractor uses), drops any range that
  crosses an inserted separator, and maps the remaining offsets back onto the
  published `formatted.plain_text`. `link_ranges_for_message_projection` uses it
  for formatted bodies; plain bodies keep `extract_link_ranges`.
- **#1594** — `.message-media-image-ready` keeps its one-column grid but names
  an explicit `max-content` row track, and `.message-media-open` is block flow
  instead of a centred inline-grid. In WebKit an implicit `auto` row kept the
  `aspect-ratio` figure's old height across a width change, and a percentage
  block size inside an auto-sized grid row resolved against the image's
  intrinsic attributes, so the frame and the image drifted apart until the row
  was re-rendered.
- **#1598** — canon only, no code.

## Verification

RED first, then the same check GREEN:

- `crates/koushi-state` — `keyword_normalization_refuses_non_ascii_case_variants`
  (refused no accented duplicate before the fix, `Ok("CAFÉ")`).
- `crates/koushi-sdk` — `planning_refuses_non_ascii_case_variants_of_a_keyword`.
- `crates/koushi-state/tests/invite_workflow_state.rs` —
  `invite_target_query_folds_unicode_case_and_width_and_sorts_with_cjk_keys`
  (0 candidates, then the folded matches in CJK order).
- `crates/koushi-core/src/timeline/item_projection/formatted_link_ranges_tests.rs`
  — adjacent `<li>`, `<p>`/`<br>` with and without whitespace, CJK + non-BMP
  UTF-16 offsets, and a run split by inline markup.
- `apps/desktop/src/components/timeline/TimelineMessageBody.test.tsx` — the
  renderer linkifies both URLs of adjacent list items from Rust-shaped ranges.
- `apps/desktop/e2e/media-attachments.spec.ts` —
  `panel resize keeps image frames in step with their images` (WebKit: image
  height 267 against a 200px frame on `main`; Chromium was already consistent).

User guide: `docs/help/settings.md` already promises case-insensitive keyword
matching and a refusal for a word already in the list, the invite pages describe
the flow without promising a matching scope, and no help page describes image
preview geometry, so no user-guide page needed a change.

Green gates: `cargo fmt` (touched crates), `cargo clippy -p koushi-state -p
koushi-core -p koushi-sdk --all-targets -- -D warnings`, `cargo test -p
koushi-state`, `-p koushi-core`, `-p koushi-sdk --lib`, desktop `tsc --noEmit`,
`eslint` + docs checks, `vitest run`, and the Playwright suites in both
chromium and webkit.

## Not covered

- WebKitGTK and WebView2 were not exercised directly; Playwright WebKit covers
  the macOS engine family and Chromium stands in for the other two.
- No live-homeserver run: all five surfaces are state projection or
  presentation, and the keyword rules were checked against the SDK's own
  matcher semantics rather than a running Synapse.
