# Attachment Fit modal

User request: show attachment staging over the entire app window, keep the image
in Fit mode, and open actual-size inspection when the image is clicked.

Consulted AGENTS.md, REPOSITORY_RULES.md, architecture/i18n.md, the media ownership
sections of architecture/overview.md and architecture/state-machine.md, and
agents/verification.md. This changes ephemeral presentation only; prepared bytes,
output selection, captions, and send commands retain their existing Rust owners.

- Render the shared main/thread staging dialog in the existing body-level floating
  layer with viewport-sized geometry, internal fallback scrolling, and focus containment.
- Remove the inline Fit/100% switch. Fit the whole image in the available preview
  row; keep output choices and caption beneath it.
- Open a native modal dialog in the top layer for actual-size prepared-image
  inspection. It scrolls on both axes, closes with Escape or the close button,
  and restores focus to the preview trigger. Closing it does not clear staging.
- Respect handled Tab events so the shared caption editor can focus Send without
  the modal focus trap immediately wrapping to the first control.

Verification:

- RED: the updated component test and headless attachment geometry test failed
  against the inline, non-modal implementation.
- GREEN: 96 tests across dialogs, panes, rightPanel, IME primitives, and CSS contracts.
- GREEN: 12 focused Playwright cases covering main/thread at 800/640/520px,
  single/multiple attachments at 360px, actual-size scrolling and focus restoration,
  attachment sending, paste/drop staging, and independent output controls.
- 13 existing media attachment Playwright cases passed during integration.
- Frontend production build, typecheck, lint, IME inventory/checker tests, and diff whitespace passed.
- Synthetic browser screenshots inspected for Fit and actual-size layout.

Local follow-up: transferred only the five UI/test files to origin/main d2153d0b
(version 0.8.1) for the user-requested installed-app replacement. The focused
57 component/CSS tests and all 9 geometry/popup Playwright cases passed here.
Native release app build completed with the local Developer ID signature.
Signature validation and installed executable hash comparison passed. The old app
was backed up, the installed app replaced, and the new process relaunched and
confirmed running. Account data was untouched. PR/merge was deferred at this stage
for local-app feedback.


## Native titlebar follow-up

The installed macOS app exposed native traffic-light buttons above both upload
staging and the timeline media viewer. The user chose top spacing, preserving
standard window controls. A shared titlebar height now reserves the macOS-only
safe area for staging, the actual-size dialog, and the timeline viewer; their
remaining height still fits the window. This is CSS presentation only.

RED: macOS staging began at 12px and the media toolbar at 0px, both inside the
44px native titlebar region. GREEN: all 25 related Chromium browser tests and
4 focused WebKit cases passed. WebKit also exposed pointer-trigger focus
restoration on popup close; explicitly focusing the preview trigger fixes it,
with the same test passing in both engines. Component/CSS tests and lint passed.

The signed local app was rebuilt, verified, backed up and replaced again with
the titlebar spacing correction. The installed executable matches the build;
relaunch was confirmed. The user approved the local app and requested a PR through
merge.

## PR integration verification

Rebased the isolated change onto main 94299d9d. Self-review confirmed that the
changes remain presentation-only and retain the existing Rust command targets.
Full frontend typecheck, 1,328 Vitest tests, lint, and secret scan passed.

The first full browser run passed 305 tests and exposed one outdated locator:
the thread upload test searched inside the context panel after staging moved to
the window-level portal. Updated the locator to the named dialog; all existing
assertions for the thread root, account, and draft revision remain unchanged.
The focused regression then passed. Full `koushi-state` and `koushi-core` tests
also passed (1,883 tests, 9 ignored). The policy's `koushi-auth` package no
longer exists; authentication coverage is part of the current Core suites.

The final full browser run and disposable Tuwunel/Synapse media QA were started
for merge verification; their final results are recorded in the PR.
