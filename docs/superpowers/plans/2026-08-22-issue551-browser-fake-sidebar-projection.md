# Issue #551 Browser Fake Sidebar Projection

## Scope

Move the final pure browser-fake sidebar projection seam from `browserFakeApi.ts` to one private leaf. This is ownership extraction, not line-count splitting: the leaf owns default sidebar construction and the fake-specific global-DM ordering correction.

Baseline `05b2f7c6`: `browserFakeApi.ts` 5,995 lines / 202,870 bytes / SHA-256 `a3f2a5db708fa51b6341255dcb8b7df0cb4035d9971f28a3eae3f192638daf55`.

## Exact move

Create private `apps/desktop/src/backend/browser-fake/sidebar.ts` and move exactly, without body/token changes:

1. `emptySidebar`
2. `composeBrowserFakeSidebar`

Export exactly those two declarations. Preserve their inferred return types rather than adding a fourth `DesktopSnapshot` type dependency or altering the moved declarations. The leaf directly imports:

- `composeSidebar` from `../../domain/desktopModel`;
- `computeBrowserRoomListProjection` from `../roomListProjection`;
- `import type` for `RoomNotificationSettings`, `RoomSummary`, and `SpaceSummary` from `../../domain/types`.

The parent removes only `composeSidebar` from its existing desktop-model import and adds one direct two-name import from `./browser-fake/sidebar`. `computeBrowserRoomListProjection` remains parent-imported because the class/snapshot factories still call it.

## Boundaries

- No class, wrapper, callback bag, registry, fixture movement, barrel, glob, compatibility shim, new state, or lifecycle owner.
- No public API/DTO/export change outside the private leaf.
- Keep static `spaces`, `rooms`, `invites`, timeline messages, thread replies, and saved sessions in the composition root; they are cross-feature fixture registries.
- Keep all `BrowserFakeApi` methods/15 fields and request/composer/prepared-byte/submission teardown in the composition root.
- Keep search/activity/threads/files/snapshot helpers in the parent; extracting them would require fixture bags or reverse dependencies.
- Combined line count may grow slightly from imports; success is narrower sidebar ownership/conflict scope, not fewer total lines.

## Deterministic verification

From immutable baseline, use TypeScript AST declarations keyed by kind/name:

- declaration slices2/2 and bodies/tokens exact;
- parent occurrences0 for both declarations;
- leaf declaration order exact and exports exactly2;
- parent has one two-name direct import and four unchanged calls total (`emptySidebar`2, `composeBrowserFakeSidebar`2);
- parent class method signatures205, fields15, top-level public exports, fixtures, timers/maps, and all non-moved production declarations exact;
- no duplicate declaration, barrel, default export, or source concatenation.

Run the same focused browser fake/client checks before and after, then full review/matrix/CI.

## Implementation evidence

- Immutable-baseline focused `browserFakeApi.test.ts`143 + `client.test.ts`25 = 168/168; the same two-file command passed168/168 x3 post-move; typecheck/lint/diff green.
- Exactness verifier: bodies/parameters2/2 exact, parent definitions0, leaf order/exports2, one direct import, calls4, class methods205/fields15 and parent exports exact, no glob.
- Metrics: parent5,995→5,940 lines; leaf62; combined6,002 (+7 lines / +313 bytes from explicit imports/exports).
- Post-implementation full-diff review: `reviewer-flash` `Correct-to-merge`; no blocking findings.
- Final local matrix: Vitest1,429, Playwright248, workspace all-targets, Tauri149/1 ignored plus keyring5, Headless Core QA130, wasm state/search, typecheck/lint/build, SDK/docs, Tauri/domain/IPC boundaries, secret/release/version, rustfmt, `cargo deny`, `cargo machete`, exactness/diff checks green. The first Playwright attempt used an out-of-tree symlinked `node_modules`, so Vite correctly denied font files outside `server.fs.allow` (247 pass/1 font-network failure); after installing the same dependency tree at the worktree path, the exact test and complete248 rerun passed.

## Residual architecture decision

After this move, no other clean browser-fake seam remains. The class is the lifecycle composition root; fixture registries and remaining helpers are cross-feature inputs. The final residual audit evidence must be recorded on #551 rather than creating line-count-only leaves.

## Gates

- `reviewer-flash` design verdict: `Correct-to-implement`; no blocking findings. The deliberate inferred return types and `import type` convention are recorded above.
- Move-only implementation and exactness verifier.
- `reviewer-flash` full-diff `Correct-to-merge`.
- Full local matrix, CI 7/7, latest-main merge, #551 browser checkbox/residual evidence, cleanup.
