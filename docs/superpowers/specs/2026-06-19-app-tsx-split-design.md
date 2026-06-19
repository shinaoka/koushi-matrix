# #87 Phase 2d — split `apps/desktop/src/App.tsx` (mechanical, behavior-preserving)

Date: 2026-06-19. Part of the #87 modularization track (the frontend counterpart to the
Rust Phase 2a/2b/2c per-feature splits). Goal: break the 6746-line `App.tsx` into a
slim Tauri-integration host plus per-component modules, with **no behavior change**.

## Constraints (from AGENTS.md + Phase 0 lints)
- `App.tsx` is a main-agent integration point. The orchestrator owns the `App.tsx`
  surgery and the shared-module extraction; only self-contained new-file creation is
  delegable.
- ESLint `no-restricted-imports` bans `@tauri-apps/*` in component files. `App.tsx` is
  the grandfathered host (3 disabled lines: `invoke`, `listen`, `getCurrentWindow`).
  Therefore everything touching Tauri (`api`, `invoke`, the two transports,
  `getCurrentWindow`, all `invoke(...)` handlers) **stays in `App.tsx`**.
- TS `typecheck` is the dependency oracle: after moving, fix imports until clean.

## Facts established by scan
- `App()` (lines 832–2834) holds ALL `invoke(...)` calls (last at 2004), `api`
  (`createDesktopApi()`), both transports, and central state/dispatch. It STAYS.
- The 31 components below `App()` (2835–6746) are **invoke-free**, reference `api.`
  **zero** times, and take explicit typed props. `t()` is a module-level import
  (`./i18n/messages`) any file can re-import.
- Only `main.tsx` imports from `App.tsx`, and only `{ App }`. The other 4 `export`s
  (`TopBar`, `WorkspaceRail`, `Composer`, `ContextualRightPanel`) have no external
  importers, so moving them breaks nothing.
- Module-level items used by components (must move to a shared module): `ICON_SIZE`
  (71×), `EMPTY_ROOM_TAGS`, `EMPTY_MENTION_INTENT`, `ignoreComposerKeyAction`,
  `formatUploadBytes`, `formatUploadDimensions`, `captionBody`, `mediaGalleryItemLabel`,
  and the types `OpenContextMenu`, `PrimaryView`, `ComposerModeProp`, `MentionCandidate`,
  `ImageCompressionPlan` (+ their transitive type deps: `ContextMenuTarget`,
  `StagedUploadItem`, `ImageCompressionVariant`, etc. — whatever typecheck requires).

## Target layout
1. **`src/app/uiShared.ts`** (NEW, pure, NO `@tauri-apps`): the shared constants/types/
   helpers listed above + their transitive type deps. Imported by `App.tsx` and the
   component files. (Keeps all Tauri/upload-flow-only helpers — `preparedImageUploadFromChoice`,
   `isImageCompressionCandidate`, `imageCompressionShouldSkip`, transports, etc. — in
   `App.tsx`, since only `App()` uses them.)
2. **`src/components/` files** (NEW), grouped by cohesive cluster (verbatim component
   bodies, only added imports differ):
   - `dialogs.tsx`: CreateEntityDialog, ImageCompressionDialog, UserIdDialog,
     ReportReasonDialog, InviteUserDialog, UploadStagingDialog
   - `Shell.tsx`: TopBar, WorkspaceRail, RoomListFilterTabs, Sidebar, RoomSection,
     NavButton, SectionTitle, RoomButton, EntityAvatar
   - `panes.tsx`: ActivityPane, ExplorePane, InvitesPane, TimelinePane, SummaryTile
   - `auth.tsx`: RecoveryPanel, AuthScreen
   - `mediaLists.tsx`: MessageArticle, RoomMediaGallery, MediaViewer,
     ScheduledMessagesList, PinnedEventsList, SearchResults
   - `composer.tsx`: Composer, ThreadComposer
   - `rightPanel.tsx`: ContextualRightPanel, PanelHeader
   (Grouping = fewer import headers / lower error surface than 31 files; intra-cluster
   call sites like Sidebar→RoomSection→RoomButton stay in one file.)
3. **`App.tsx`** keeps: all imports (incl. the 3 grandfathered `@tauri-apps`), `api`,
   transports, `invoke` handlers, the Tauri-only module-level helpers, `App()`, and the
   `export function App`. It gains `import` lines for `uiShared` and the new component
   modules; the moved component/shared bodies are deleted.

## Cross-file ordering / hazards
- Components calling other moved components (Sidebar→RoomSection/RoomButton/NavButton/
  SectionTitle, TimelinePane→MessageArticle/Composer, etc.): grouping co-locates the
  common chains; cross-file references import the sibling component. No circular value
  imports because `uiShared` holds the shared leaf items, not components.
- No component imports `@tauri-apps` (verified invoke-free); the eslint boundary stays
  green without new disables.
- `MessageArticle`/`Composer` may reference already-split modules (`TimelineView`,
  `RoomInfoPanel`); keep those imports.

## Execution
1. Orchestrator writes `uiShared.ts` and the exact per-file extraction spec (component
   line ranges + the precise import header each file needs).
2. deepseek-v4-flash creates the component files (verbatim bodies + specified imports);
   it does NOT touch `App.tsx`.
3. Orchestrator does the `App.tsx` surgery (delete moved bodies/shared items, add the
   new imports) and runs `typecheck`, fixing any import gaps.
4. Verify (orchestrator, real exit codes): `npm run typecheck`, `npm test` (vitest),
   `npm run build`, `npm run lint` (+ tauri-boundary/domain-deps), and Playwright
   `e2e/basic-operations.spec.ts` (+ the known-flaky reply/pin specs in isolation).
5. codex diff review (pure-move verification), PR, CI-gated auto-merge.

## Proof of behavior-preservation
- No `.tsx`/`.ts` logic edits — only moves + import wiring. Component bodies byte-identical.
- `typecheck` + `vitest` (366 tests) + Playwright basic-operations + eslint boundary all
  green; `App.tsx` still the sole Tauri host (boundary unchanged).
