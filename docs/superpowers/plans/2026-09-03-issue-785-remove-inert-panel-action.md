# Issue #785 — remove inert right-panel More action

## Scope and acceptance

- Shared right-panel headers expose no inert More button or focus stop in any panel mode.
- Existing Close buttons retain their labels, visibility policy, click behavior, keyboard activation, and panel-specific content.
- Working More/context-menu buttons elsewhere (room header, timeline/media actions) remain untouched.
- Remove the now-unused `action.more` catalog key; do not leave a dead token.

## Root cause and decision

`PanelHeader` unconditionally renders a real More button with an accessible name but no handler, menu state, descriptor, or caller-owned action. All contextual panel modes inherit the false affordance. No current acceptance requirement defines per-panel overflow actions.

Delete the placeholder rather than creating speculative menus or optional abstractions. A future contextual action must arrive with a concrete owner, handler, accessible semantics, and tests. The renderer-only removal does not alter Rust product state, DTOs, Tauri, SDK, or navigation settlement.

## Verify first

- Add a component test for shared `PanelHeader` proving it renders exactly the requested title and Close control, invokes Close by click/keyboard path, and has no More button.
- Add a browser assertion with User settings open proving the contextual panel has Close but no `button[name="More"]`, so the real shared composition and accessibility tree are covered.
- Run before removal and record RED, then unchanged GREEN.

## Minimal implementation

- Remove the handler-less More button and its `MoreHorizontal` import from `rightPanel.tsx`; remove the same dead control from the tracked `apps/desktop-shell` prototype mirror.
- Reduce `.thread-header` from the stale title/More/Close three-column grid to title/Close (`minmax(0, 1fr) auto`) in both current desktop and prototype styles.
- Remove the sole-use `action.more` key from the TypeScript key union and both English/Japanese catalogs.
- Correct only `docs/superpowers/specs/2026-06-11-matrix-desktop-design.md:199` so panel headers provide Close and only concrete caller-supplied contextual actions, never placeholders. Retain line 160's working message-hover “more actions where supported” contract.
- Do not introduce a menu descriptor, no-op handler, hidden/disabled button, or compatibility shim.

## Gates

Focused right-panel and browser tests; i18n tests; full Vitest; relevant Playwright; typecheck, lint, build, format/diff, SDK/submodule, secret/boundary/docs checks, required Rust package tests, GitHub CI, merge, Issue close, and main CI.

## Review record

- Design review: `reviewer-flash` **Correct-to-merge**. Its two Minor clarifications were incorporated: remove the stale third header grid column and pin the exact design-spec sentence while retaining working message-hover actions.
- Verify-first evidence: component and browser checks both failed before removal because More was present; unchanged checks passed after removal. Focused right-panel+i18n Vitest passed 31/31; full Vitest passed 99 files/1215 tests; full viewport browser spec passed 3/3. Core passed 907/8 ignored, SDK 143, desktop 116, state 40; typecheck, lint, build, format/diff, SDK-submodule, secret, and dead-mirror checks passed.
- Implementation review Round 1 timed out without a verdict after identifying the tracked `apps/desktop-shell` prototype's dead More mirror. That mirror and its stale grid column were removed; the review's assertion that current E2E still failed referred to the recorded pre-fix RED, while the post-fix command passed. Final re-review: **Correct-to-merge**, no remaining findings.
