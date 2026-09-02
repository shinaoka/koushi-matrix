# Issue #779 — main-composer Tab focuses Send first

## Scope and acceptance

For the editable main timeline composer only:

- A sendable draft plus unmodified forward Tab from the editor focuses enabled Send.
- Enter or Space on focused Send uses its existing button click path exactly once.
- Mention-autocomplete Tab continues accepting the active candidate first.
- Shift+Tab, modified Tab, and IME-composing Tab remain native/unintercepted.
- Disabled Send is skipped; the first available auxiliary control receives native focus.
- After enabled Send, continued forward Tab reaches attachment, mention, emoji, and scheduled-send controls.
- Existing attachment-caption Tab-to-Send remains intact; thread composer behavior is unchanged.

This is renderer-local keyboard focus policy. Rust continues owning draft/send eligibility, composer intent resolution, and send settlement; no DTO/Tauri/SDK/i18n changes are required.

## Existing seam and root cause

`Composer.onComposerKeyDown` already gives mention autocomplete precedence and then provides an opt-in `onTabToSend` hook guarded against Shift/Ctrl/Alt/Meta and IME composition. Attachment captions use that seam. The main `Composer` in `TimelinePane` does not opt in, so native DOM order reaches auxiliary controls before Send.

Merely focusing Send from the editor would make subsequent forward Tab leave the composer because Send is currently last in DOM order. Positive `tabIndex` is forbidden because it creates a second global focus order. The main surface therefore needs both the existing guarded focus seam and a main-only semantic DOM order of Send followed by auxiliary controls, while CSS preserves the current visual order.

## Verify first

1. Add non-vacuous component behavior coverage for an opt-in `Composer`:
   - sendable unmodified Tab focuses Send;
   - mention-open Tab accepts a candidate instead;
   - Shift/modified/composing Tab do not invoke the focus seam;
   - disabled Send Tab is not prevented and focus remains on the editor (jsdom does not simulate native traversal);
   - Send precedes the auxiliary wrapper in DOM and the opt-in hidden file input has `tabIndex=-1`.
2. Add a browser regression in the real harness main timeline composer that performs actual native focus travel. Use separate states: (a) type a draft, Tab to Send, and activate it exactly once; (b) type a fresh unsent draft and traverse Send → attachment → mention → emoji → scheduled send; (c) use an empty draft to prove disabled-Send fallback. Component tests assert only jsdom-observable policy/DOM proxies; browser tests own all native Tab traversal assertions.
3. Run the focused tests before production changes and record RED, then unchanged GREEN.

## Minimal implementation

- Add an opt-in `preferSendOnForwardTab` prop to shared `Composer`.
- Let `Composer` own a Send button ref. Its effective `onTabToSend` remains the existing external callback when supplied; otherwise, only when opted in with `surface === "main"` and `!editorOnly`, it focuses Send when the ref exists and the same conditions that enable Send hold (`canEdit`, not sending, and `localValue.trim().length > 0`, using the same local draft source as the button). When ineligible it is absent, so the existing key path does not prevent Tab. The interception makes the main-surface policy explicit; semantic DOM order independently makes continued native Tab reach the auxiliary controls.
- For the opt-in surface only, render Send before an auxiliary wrapper with class `.composer-footer-controls` in DOM order. Set `.composer-footer-controls { order: 1 }` and `.send-button { order: 2 }`, retaining current auxiliary-left / Send-right layout. The existing thread footer class is untouched.
- Remove the hidden file input from sequential focus only when opted in (`tabIndex={preferSendOnForwardTab ? -1 : undefined}`); the visible attachment button remains the accessible chooser trigger and thread focus behavior is unchanged.
- Pass the opt-in only from the main `TimelinePane` composer. Do not enable it for thread, inline-edit, or attachment-caption composers.
- Do not add document-level handlers, positive `tabIndex`, timing, duplicate key parsing, or Rust state.

## Verification and gates

Focused component and browser tests; attachment-dialog tests; full Vitest and relevant Playwright; typecheck, lint, build, format/diff, SDK-submodule, secret/boundary/doc checks; Rust workspace package tests required by repository preflight; GitHub required CI.

## Review record

- Design review Round 1: one Important test-non-vacuity finding and four Minor specification findings. The plan now assigns native traversal to Playwright, uses jsdom-observable proxy assertions, scopes file-input behavior to main only, pins exact CSS ordering, documents eligibility/no-prevent behavior, and guards `editorOnly`.
- Design re-review: `reviewer-flash` **Correct-to-merge**. Three additional Minor refinements were incorporated: `localValue` eligibility, browser-accurate Enter/Space activation shape, and separate unsent traversal state.
- Verify-first evidence: focused Playwright failed before production changes because Send remained inactive after editor Tab; unchanged gate then passed. Focused Chromium tests cover semantic/visual order, disabled fallback, auxiliary traversal, Enter exact-once, and Space exact-once. Composer plus attachment-dialog Vitest passed 77/77; full Vitest passed 99 files/1214 tests. Focused browser plus shell-a11y passed 3/3. Core passed 907/8 ignored, SDK 143, desktop 116, state 40; typecheck, lint, build, format/diff, SDK-submodule, and secret checks passed.
- Implementation review: `reviewer-flash` **Correct-to-merge** with two Minor findings. Both were fixed: native Enter/Space exact-once is specified only in Playwright, and the opt-in is hard-guarded to `surface === "main"` plus non-editor-only. Final re-review: **Correct-to-merge**, no remaining findings.
