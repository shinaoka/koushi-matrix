# Issue #835 deterministic README screenshot

Status: design approved and implemented. reviewer-flash design round 1 required F1–F8 corrections; round 2 was Correct-to-merge. The canonical-divider/Shell scope correction was accepted before final implementation. reviewer-flash full-diff review reported no Critical or Important findings and Correct-to-merge, subject to the one-time visual gate completed below.

## Outcome

Generate the single README product screenshot from the real React `<App />` in the existing Rust-DTO-shaped browser harness. The image must let a visitor recognize an Element-like three-pane Matrix desktop client within about two seconds. Generation is deterministic in pinned Linux headless Chromium, contains only synthetic data, and never exercises a real homeserver or native Tauri window.

The checked-in PNG is updated only by an intentional developer command. CI regenerates and compares it but never commits, pushes, schedules updates, or silently normalizes pixel differences.

## Fixed composition

- Capture a 1280×800 CSS viewport at device scale factor 2 as a 2560×1600 PNG; README renders it at width 800.
- Light theme, no dialog, no hover, no focus, right panel closed.
- Space rail: Home plus `Lattice Lab` selected, `Photon Reading Group`, and `Release Crew`, all initials avatars.
- Sidebar: four rooms (`General` selected/encrypted, `Design` unread 3, `Papers` favourite, `Random`) and DM `Aki`.
- Header: the production-renderable `General` room name and its existing header navigation/actions; no invented topic, member-count, or encryption badge (those fields are not rendered by the closed-panel product header), warnings, or runtime alerts.
- Timeline: exactly one date divider and nine fixed messages from Aki, Ren, Sora, and `You (Koushi)` across about 40 minutes. Include consecutive sender grouping, one reply quote, one thread root with three-reply summary, reactions `👍` ×2 and `🎉` ×1 with one own reaction, and one edited marker.
- Empty unfocused composer with placeholder. No media/avatar images, URLs, previews, UTD, pending/failed sends, redactions, code blocks, spoilers, caret, reply/edit banner, or popup.

All Matrix IDs use `example.invalid`; homeserver is `https://harness.example.invalid`. No fixture identifier, runner path, or private-looking value may be visible.

## Fixture and production-surface boundary

Add `apps/desktop/e2e-docs/readmeFixture.ts`, typed with existing `DesktopSnapshot` and `TimelineItem` domain contracts. Start from `window.__harness.currentSnapshot()`, override only authoritative Rust-shaped domain/sidebar/settings projections, call existing `pushStateUpdate()`, then deliver `InitialItems` through existing `pushCoreEvent()` exactly as normal browser-headless tests do.

Keep all nine fixed timestamps within 2026-03-10 UTC and prepend exactly one existing canonical synthetic date-divider `TimelineItem`, copied from the established `App.test.tsx`/`TimelineItemRow` fixture shape (`Synthetic.synthetic_id = date-divider-1773133200000`, null sender/body, `timestamp_ms = 1773133200000` for 2026-03-10T09:00:00Z, no actions/reactions). The fixture supplies the canonical divider explicitly for exactness; do not change or depend on product projection logic for the screenshot or invent a new DTO shape. The GREEN test asserts its rendered label matches the message date. The `InitialItems.key` must exactly equal `roomTimelineKey(snapshot.state.domain.session.user_id, snapshot.state.ui.timeline.room_id)`. Publish the state update before `InitialItems` so App has retained that key, and do not replace the harness session identity because that resets the timeline store. Do not modify `appHarnessMain.tsx`, `Shell.tsx`, timeline projection/product components, add React state, duplicate product rendering, or make native Linux GUI QA a dependency.

## Determinism contract

- Add `apps/desktop/playwright.docs.config.ts` with `testDir: "./e2e-docs"`, viewport 1280×800, `deviceScaleFactor: 2`, locale `en-US`, timezone `UTC`, light color scheme, reduced motion, one worker, zero retries, and Vite port 5184. Add `e2e-docs/**` to Vitest's `test.exclude` so normal frontend tests never collect Playwright specs.
- Pin local and CI generation to `mcr.microsoft.com/playwright:v1.60.0-noble`, matching resolved `@playwright/test` 1.60.0. The spec/config must assert that package version and fail loudly after a lockfile bump. The documented manual bump procedure updates the image tag, regenerates twice, and re-verifies byte identity.
- Fixture sets `settings.values.appearance.theme = "light"`, selects bundled Inter and Twemoji COLR in both `settings.values.typography` and `domain.typography_profile`, and does not rely on the harness `system` default. Before capture, await `document.fonts.ready`; assert Inter and Twemoji faces are loaded.
- Blur the active element and never move the mouse. Capture with `animations: "disabled"` and `caret: "hide"`. If the unchanged harness homeserver label would expose `example.invalid` in the top status strip, hide only `.sync-status-server` with a documentation-spec `page.addStyleTag`; do not change `Shell.tsx` or production label behavior.
- Use fixed 2026-03-10 UTC timestamps and Latin message bodies; do not populate `recency_stamp`, `conversation_activity`, or any fixture value from `Date.now()`. Guard visible output against current-wall-clock text and forbidden fixture/ID/path strings. After state/event delivery, wait on the existing timeline layout/virtualization settlement pattern before capture.
- Do not add pixel tolerance. Two same-commit container runs must produce byte-identical SHA-256. If they differ, fix the leaked font/time/layout source.

## Files and command

- `apps/desktop/e2e-docs/readme-screenshot.spec.ts`
- `apps/desktop/e2e-docs/readmeFixture.ts`
- `apps/desktop/playwright.docs.config.ts`
- package script: `npm --prefix apps/desktop run docs:screenshot`
- output: compute the repository-root path in the spec with `fileURLToPath(new URL("../../../assets/screenshots/koushi-main.png", import.meta.url))`; never resolve it from the `apps/desktop` process cwd
- create `.gitattributes` with `*.png binary`
- `README.md`: insert the image immediately after the wordmark paragraph's closing `</p>` and before the existing `A desktop client…` prose, with descriptive alt text and `width="800"`; developer documentation gives the pinned-container regeneration command and lock/image-version bump procedure
- `docs/agents/qa-lanes.md`: documentation screenshot lane, explicitly not native GUI/IPC proof

## Verification-first sequence

1. RED: add the focused screenshot spec and Vitest exclusion, then run it before the fixture/output wiring is complete. It must fail on the missing authoritative populated state/artifact or required composition assertion; preserve this output.
2. GREEN: generate once in the pinned container and assert exact dimensions, required visible labels/features, closed right panel, empty composer, loaded bundled fonts, and absence of forbidden visible data.
3. Generate twice from the same clean submitted state in separate pinned-container invocations; compare SHA-256 hashes and require byte identity.
4. Run the existing browser-headless suite to prove the separate `e2e-docs` directory/config and port do not alter shared harness behavior.
5. Run frontend typecheck, lint, build, secret scan, agent-doc structure checks, and `git diff --check`.
6. Inspect the actual committed PNG at README display scale for three-pane readability, clipping, correct selection/unread/favourite/DM, date divider/reply/thread/reactions/edited marker, empty composer, Inter glyphs, and synthetic-only content.

## CI contract

Add independent `readme-screenshot` job to `.github/workflows/ci.yml`, using the pinned Playwright container. It verifies the resolved Playwright package is exactly 1.60.0, installs exact lockfile dependencies, runs `npm run docs:screenshot` from `apps/desktop`, then checks the repository-root path from the checkout root:

```sh
test -d "$GITHUB_WORKSPACE/.git"
git -C "$GITHUB_WORKSPACE" diff --exit-code -- assets/screenshots/koushi-main.png
test -z "$(git -C "$GITHUB_WORKSPACE" status --porcelain -- assets/screenshots)"
```

On mismatch it may upload the generated PNG only as a debugging artifact. It must never commit/push and must not depend on GUI, homeserver, or native IPC jobs.

## One-time image review checklist

- Three panes readable at README width; no right/bottom clipping.
- Selected space and room, unread badge, favourite, and DM visible.
- Date divider, reply quote, thread summary, reactions, and edited marker visible.
- Composer empty and caret-free.
- Bundled Inter rendering is evident.
- No `Harness Room`, `example.invalid`, runner path, dialog, warning, or private data visible.

## Verification evidence

Two independent runs in `mcr.microsoft.com/playwright:v1.60.0-noble` generated the identical SHA-256:

```text
run 1  0db4d5a37eb6537829ce29e7b2488caf6c4ca62e0a4d193ee11ccb9c31de525d
run 2  0db4d5a37eb6537829ce29e7b2488caf6c4ca62e0a4d193ee11ccb9c31de525d
```

The committed PNG is 2560×1600. Image-capable inspection confirmed readable three-pane composition at README scale, no right/bottom clipping, selected space/room, unread/favourite/DM rows, the March 10 date divider, reply, thread summary, reactions, edited marker, empty unfocused composer, closed right panel, bundled-font rendering, and no visible harness IDs, `example.invalid`, runner path, or private data.

## Exclusions

No dark variant, gallery, GIF, thread panel, CJK sample, native window chrome, scheduled update, automated commit, tolerance-based comparison, screenshot-specific replica UI, or new React product state.
