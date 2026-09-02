# Issue #778 — contain User settings quick-navigation scrolling

## Scope and acceptance

- At a 1334×852 viewport, every User settings quick-navigation action scrolls only `.settings-panel`.
- `.desktop` and the title bar remain at top 0; no hidden shell scroll range is created while the panel is open.
- Wheel/trackpad scrolling, panel close, density selection, keyboard/accessibility semantics, and responsive layout remain unchanged.
- This is DOM layout/navigation behavior. No Rust-owned product state, DTO, Tauri, SDK, or i18n contract changes.

## Root cause and ownership boundary

`UserSettingsPanel.scrollToSection` calls `scrollIntoView`, which may scroll every scrollable ancestor. `.desktop` is an `overflow: hidden` box, but remains programmatically scrollable and gives users no scrollbar with which to restore it. Because `.settings-panel` is not positioned, absolutely positioned descendants can resolve their containing block outside the panel at `.app-grid`. The containment fix prevents such descendants from contributing geometry outside the intended scroll boundary; it does not depend on attributing the full observed range to one specific element.

The renderer owns this local DOM viewport behavior, but only the settings panel may own settings-section scroll position. The shell must remain viewport-aligned.

## Verify first

Extend `apps/desktop/e2e/viewport-layout.spec.ts` with a deterministic 1334×852 browser check that:

1. opens User settings and proves `.desktop.scrollTop === 0` and the title bar top is 0;
2. activates each quick-navigation button;
3. activates General → Security → General and proves the panel scroll position strictly increases and then returns, making the scroll oracle non-vacuous;
4. proves `.desktop.scrollTop` and title-bar top remain 0 after every activation;
5. proves `.desktop.scrollHeight === .desktop.clientHeight`, so no latent script-scrollable shell range exists.

Run it before the fix and record RED from shell displacement/overflow, then run the identical test GREEN.

## Minimal implementation

1. Make `.settings-panel` positioned (`position: relative`) so absolute descendants such as the avatar file input use the panel as their containing block and are clipped by its existing `overflow: auto` boundary.
2. Replace `scrollIntoView` with an explicit instant assignment of `panelRef.current.scrollTop` to the target section's panel-relative `offsetTop`. This prevents any current or future ancestor from being scrolled as a side effect and avoids smooth-scroll timing.
3. Do not add global shell reset handlers, scroll event interception, timeouts, or viewport-sync/native changes; those would repair symptoms after violating ownership.

## Verification and gates

- Focused Playwright test, including the short viewport.
- Existing `viewport-layout.spec.ts` fully.
- UserSettingsPanel/Vitest suite, full Vitest, typecheck, lint, build, format/diff, secret and repository boundary/doc checks.
- GitHub required CI, with browser headless and platform checks green.
- Independent design and final diff reviews recorded here.

## Review record

- Design review: `reviewer-flash` **Correct-to-merge**. Three Minor suggestions were incorporated above: mechanism wording, non-vacuous bidirectional assertions, and explicit instant panel scrolling.
- Verify-first evidence: focused Chromium test failed before the fix with `.desktop.scrollTop` 56 instead of 0, then passed unchanged after the fix. The complete viewport spec passed 2/2; the focused WebKit run also passed. UserSettingsPanel Vitest passed 25/25 and full Vitest passed 99 files/1211 tests; Core passed 907/8 ignored, SDK 143, desktop 116, and state 40; typecheck, lint, production build, format/diff, SDK-submodule, and secret checks passed.
- Implementation review: `reviewer-flash` **Correct-to-merge** with two Minor findings. Both were fixed: the CSS now documents its load-bearing offset-parent/containment role, and the root-cause wording no longer attributes the full latent range to one element. Final re-review: **Correct-to-merge**, no remaining findings.
