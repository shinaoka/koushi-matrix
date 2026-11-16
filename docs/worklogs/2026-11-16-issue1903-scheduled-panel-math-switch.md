# Scheduled-messages panel: the editor's Σ switch (2026-11-16, #1903)

The reporter's recording shows the Σ switch in the right panel's scheduled-send
editor drawn blue, "Math formatting on", unresponsive to several clicks, while
the main composer's switch below it stays gray. The same editor opened from the
room timeline's scheduled item behaves. 0.35.1, macOS; the panel mount is
unchanged at `0cac3cf2` (0.35.2).

Canon consulted: `REPOSITORY_RULES.md` ("Root-Cause Fix Discipline"),
`docs/policies/engineering-rules.md` (React renders Rust DTOs; no second state
path), `apps/desktop/src/components/composer.tsx` (#453 comment on the switch),
and the #1244 / #1346 / #1348 records for the earlier surfaces of this control.
No upstream behaviour is imitated: the Σ switch is this client's own control.

## Cause

One mount, not the switch:

- `ScheduledMessagesList` takes `mathModeEnabled` / `onMathModeChange` and
  forwards them to the inline `<Composer editorOnly>` of the item being edited
  (`mediaLists.tsx:119-216`). Its Σ switch is in the composer's tools row, which
  is not gated on `editorOnly`, so the editor always shows it.
- Three mounts pass a real value: `TimelinePane` in `panes.tsx`, and — for the
  right panel's thread composer and staging dialog — `rightPanel.tsx`. Only
  `ContextualRightPanel`'s scheduled-messages mount (`rightPanel.tsx:1106`)
  passed neither, so the editor fell through to the component defaults
  `mathModeEnabled = true` and `onMathModeChange = () => undefined`: a switch
  that reads "on" and does nothing.
- The delayed content itself was already formatted in Rust from the real setting
  (#1348), so the defect was a dead control, not a formatting error.

## Fix

- `rightPanel.tsx` mounts the list with `mathModeEnabled={composerSettings.math_mode}`
  and the panel's existing app-scoped settings handler. Nothing new: no state,
  no IPC, no command. The handler was named `threadMathModeChangeStable` but
  writes `{scope: "app", composer: {...composerSettings, math_mode}}` for every
  surface in the panel, so it is now `composerMathModeChangeStable` — the name
  matched a thread-only mount that never existed.
- The two defaults in `ScheduledMessagesList` are gone and both props are
  required, which is the issue's second suggested direction: a sixth surface
  that forgets the setting is now a type error instead of a silent "on".

A sweep of every mount of every component that takes these props (`Composer`,
`ThreadComposer`, `TimelineView`, `TimelineItemRow`, `UploadStagingDialog`,
`ScheduledMessagesList`) finds this one gap as the last one. The remaining mount
without the props is `apps/desktop/src/test/harnessMain.tsx`, the browser fixture
harness, which has no settings source of its own; the DOM-tier specs that assert
the switch drive the App harness (`appHarnessMain.tsx`), which carries settings.
It is left alone.

## Evidence

- RED first: the new panel tests fail with `switch "Math formatting off" not
  found` (the editor renders its "on" default) and `onUpdateSettings` called 0
  times (the default handler is a no-op); the new DOM-tier test fails with the
  panel editor's switch absent under `math formatting off`. GREEN after the two
  mount lines.
- `scheduledMessagesPanel.test.tsx` asserts the setting reaches the switch and
  that a click sends the same app-scoped patch the main composer sends, then
  re-renders from that patch.
- `e2e/scheduled-messages-panel.spec.ts` drives the reporter's own steps in the
  mounted App: turn the composer's Σ off, open the panel, edit — both switches
  are off and there are exactly two of them — click the panel's switch, and both
  are on again with one `update_settings` invocation.
- `npm run typecheck`, `npm run lint`, `npm run test -- --run` (202 files, 2496
  tests) and the four scheduled/composer DOM-tier specs (68 tests, chromium +
  webkit) pass.

- Review (glm-5.3-flash): the thread `UploadStagingDialog` mount passed an inline
  copy of the same app-scoped patch; it now uses `composerMathModeChangeStable`
  too, so one handler serves all five panel surfaces.

## Not covered

The switch keeps no per-surface state, so no Rust-side or IPC behaviour changed
and no headless QA lane was added: the DOM tier above is the UI evidence, and the
existing `composer.math_mode` coverage in Core is untouched. Manual GUI
inspection was not needed to find or fix this, and was not done.

`Composer`, `ThreadComposer`, `TimelineView`, `TimelineItemRow` and
`UploadStagingDialog` keep their opt-in defaults. Their current mounts all pass a
real value, but a future mount that omits one would reproduce this defect class
without a type error; making them required is a separate change across a dozen
mounts, so it is recorded here rather than bundled in.
