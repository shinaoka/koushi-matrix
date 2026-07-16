# IME-Safe Text Inputs Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make IME-safe DOM ownership and candidate-confirmation submit handling a single shared contract across every desktop text input.

**Architecture:** Generalize the current composition lifecycle into a reusable text-control controller, wrap it with thin React input/textarea/secure/form adapters, and migrate all feature surfaces. A keyed async-result gate protects full-snapshot text mutations, while a TypeScript-AST audit prevents raw composable controls and forms from returning.

**Tech Stack:** React 19, TypeScript 6, Vitest/Testing Library, Node test runner, TypeScript compiler API, ESLint, Tauri snapshot API.

## Global Constraints

- Active IME composition owns DOM value, selection, and replacement range.
- Candidate-confirmation Enter keeps the native browser default and never triggers product submit.
- One lifecycle implementation serves input, search, textarea, secure, composer, and edit surfaces.
- Secret values remain DOM-only and never enter React state, snapshots, diagnostics, or logs.
- Feature components contain no raw composable input, textarea, contentEditable, or raw form after migration.
- All behavior changes follow RED → GREEN and keep #240 composer/edit regressions green.

---

### Task 1: Generalize the composition-owned DOM controller

**Files:**
- Modify: `apps/desktop/src/domain/compositionLifecycle.ts`
- Modify: `apps/desktop/src/domain/compositionLifecycle.test.ts`

**Interfaces:**
- Produces `TextControlElement = HTMLInputElement | HTMLTextAreaElement`.
- Produces `useCompositionOwnedTextControl<T extends TextControlElement>(externalValue: string | undefined, syncKey: string)`.
- Preserves `useCompositionOwnedTextarea` as a compatibility wrapper until Task 2 migrates callers.
- Produces pure dirty/ack/key-change state transitions for direct unit tests.

- [ ] **Step 1: Write failing controller tests**

Add tests proving that a locally typed value survives a stale external rerender, a matching external value acknowledges it, a later clean external value synchronizes, and a `syncKey` change forces the new field value.

```ts
const state = createCompositionOwnedValueState("before", "field-a");
state.recordLocalValue("local");
expect(state.observeExternal("before", "field-a")).toEqual({ kind: "ignoreStale" });
expect(state.observeExternal("local", "field-a")).toEqual({ kind: "acknowledged" });
expect(state.observeExternal("server", "field-a")).toEqual({ kind: "write", value: "server" });
```

- [ ] **Step 2: Verify RED**

Run `npm --prefix apps/desktop test -- src/domain/compositionLifecycle.test.ts` and expect failure because the value-state/controller APIs do not exist.

- [ ] **Step 3: Implement the minimal pure value state and generic hook**

Keep epoch handling in `createCompositionLifecycle`. The hook writes `control.value` only for an explicit `write` decision and records local values before consumer callbacks.

- [ ] **Step 4: Verify GREEN**

Run the same command and require all lifecycle tests to pass.

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/src/domain/compositionLifecycle.ts apps/desktop/src/domain/compositionLifecycle.test.ts
git commit -m "refactor: generalize composition-owned text state"
```

### Task 2: Add shared React text and form primitives

**Files:**
- Create: `apps/desktop/src/components/ImeTextControl.tsx`
- Create: `apps/desktop/src/components/ImeTextControl.test.tsx`
- Modify: `apps/desktop/src/components/composer.tsx`
- Modify: `apps/desktop/src/components/TimelineView.tsx`

**Interfaces:**
- Produces `ImeTextField`, `ImeTextArea`, `SecureImeTextField`, `ImeOwnedTextArea`, and `ImeSafeForm`.
- `ImeTextField` accepts ordinary input props plus `value?: string` and `syncKey?: string`.
- `SecureImeTextField` omits `value` and `defaultValue` and forwards a DOM ref.
- `ImeSafeForm` invokes product `onSubmit` only when no IME-commit fence is pending.

- [ ] **Step 1: Write failing parameterized adapter tests**

Cover text, search, textarea, and secure adapters: composition rerenders preserve value/selection; stale ordinary rerenders preserve dirty DOM; matching values acknowledge; key changes synchronize; secure values are read only from refs.

- [ ] **Step 2: Write the failing form test**

Start composition, send Enter with `isComposing`, submit the form, and assert the product callback is not called while the keydown default remains unprevented. After composition settles, ordinary Enter/submit calls the callback once.

- [ ] **Step 3: Verify RED**

Run `npm --prefix apps/desktop test -- src/components/ImeTextControl.test.tsx` and expect module-not-found/API failures.

- [ ] **Step 4: Implement primitives and form context**

Adapters record DOM changes before consumer handlers. IME Enter marks the nearest form fence and is not forwarded to product `onKeyDown`; it is never prevented. `ImeSafeForm` prevents only the associated submit event.

- [ ] **Step 5: Migrate composer and edit textarea**

Use `ImeOwnedTextArea` with their externally-created controller so async intent snapshots retain the same lifecycle generation. Remove raw textarea markup without duplicating lifecycle logic.

- [ ] **Step 6: Verify GREEN**

Run `npm --prefix apps/desktop test -- src/components/ImeTextControl.test.tsx src/components/composer.test.tsx src/components/TimelineView.test.tsx` and require all tests to pass.

- [ ] **Step 7: Commit**

```bash
git add apps/desktop/src/components/ImeTextControl.tsx apps/desktop/src/components/ImeTextControl.test.tsx apps/desktop/src/components/composer.tsx apps/desktop/src/components/TimelineView.tsx
git commit -m "feat: add shared IME-safe text controls"
```

### Task 3: Fix async-backed caption and alias fields

**Files:**
- Create: `apps/desktop/src/domain/latestAsyncResult.ts`
- Create: `apps/desktop/src/domain/latestAsyncResult.test.ts`
- Modify: `apps/desktop/src/components/dialogs.tsx`
- Create: `apps/desktop/src/components/dialogs.test.tsx`
- Modify: `apps/desktop/src/components/PeoplePanel.tsx`
- Modify: `apps/desktop/src/components/TimelineView.tsx`
- Modify: `apps/desktop/src/App.tsx`

**Interfaces:**
- Produces `createLatestAsyncResultGate<Key>()` with `begin(key): { isCurrent(): boolean }` and `invalidate(key)`.
- Upload caption uses `ImeTextField` with `syncKey={item.staged_id}`.
- Alias fields use user identity as `syncKey`.

- [ ] **Step 1: Write failing latest-result tests**

Begin generations A and B for one key and a generation for a second key. Assert only B and the independent key are current.

- [ ] **Step 2: Verify RED**

Run `npm --prefix apps/desktop test -- src/domain/latestAsyncResult.test.ts` and expect failure because the gate does not exist.

- [ ] **Step 3: Implement the gate and verify GREEN**

Use a private `Map<Key, number>` and monotonically increasing generations. Re-run the test.

- [ ] **Step 4: Write the failing upload-caption reproduction**

Start composition, enter synthetic Japanese text, set a selection, rerender with the old Rust caption and changed preparation object, and assert DOM value/selection remain. Repeat outside composition to prove dirty local DOM survives a stale snapshot.

- [ ] **Step 5: Verify RED**

Run `npm --prefix apps/desktop test -- src/components/dialogs.test.tsx` and expect the controlled caption to be overwritten.

- [ ] **Step 6: Migrate caption/alias fields and gate async snapshots**

Apply returned snapshots only when their logical field generation is still current.

- [ ] **Step 7: Verify GREEN and commit**

Run the caption, alias, timeline, and latest-result tests; then commit as `fix: preserve IME ownership for async text drafts`.

### Task 4: Migrate every remaining text control and form

**Files:**
- Modify: `apps/desktop/src/App.tsx`
- Modify: `apps/desktop/src/components/{SpaceInfoPanel,PeoplePanel,UserSettingsPanel,dialogs,FilesView,RoomInfoPanel,TimelineView,panes,EmojiPicker,Shell,auth,mediaLists,rightPanel}.tsx`
- Modify focused tests beside changed components.

**Interfaces:**
- Every dynamic field supplies a semantic `syncKey` such as room ID + field, user ID, staged ID, dialog instance, or draft key.
- Secret inputs use `SecureImeTextField` and refs; controlled password strings become DOM refs plus non-secret validity facts.

- [ ] **Step 1: Add failing representative surface tests**

Cover Files search candidate-confirmation Enter, RoomInfo snapshot rerender during composition, create/report submit during composition, and secure/password submit through refs.

- [ ] **Step 2: Verify RED**

Run the affected test files and confirm failures are missing shared behavior rather than selector errors.

- [ ] **Step 3: Replace raw composable controls**

Use `ImeTextField`, `ImeTextArea`, or `SecureImeTextField`; preserve CSS, ARIA, autocomplete, spellcheck, disabled, and refs.

- [ ] **Step 4: Replace raw forms**

Use `ImeSafeForm` for native submit, explicit search Enter, auth, recovery, room settings, aliases, and settings.

- [ ] **Step 5: Remove controlled secret strings**

Read secrets from refs on submit, store only booleans/lengths needed for enablement, and clear DOM values after terminal handling.

- [ ] **Step 6: Verify and commit**

Run `npm --prefix apps/desktop test`; require no controlled/uncontrolled warnings; commit as `refactor: migrate desktop text surfaces to shared IME controls`.

### Task 5: Add static enforcement and amend canon

**Files:**
- Create: `scripts/check-ime-text-inputs.mjs`
- Create: `scripts/check-ime-text-inputs.test.mjs`
- Modify: `apps/desktop/package.json`
- Modify: `REPOSITORY_RULES.md`
- Modify: `docs/policies/engineering-rules.md`
- Modify: `docs/architecture/overview.md`
- Modify: `AGENTS.md`
- Include the issue #272 spec and this plan.

**Interfaces:**
- `node scripts/check-ime-text-inputs.mjs` exits nonzero with relative file/line/kind findings.
- `npm --prefix apps/desktop run lint` runs ESLint and the audit.

- [ ] **Step 1: Write failing AST-audit tests**

Use synthetic unsafe input/search/password/textarea/contentEditable/form sources and safe primitive/non-composable sources. Assert stable finding kinds.

- [ ] **Step 2: Verify RED**

Run `node --test scripts/check-ime-text-inputs.test.mjs`; expect failure because the audit module is absent.

- [ ] **Step 3: Implement the AST audit**

Resolve TypeScript from `apps/desktop/node_modules`, scan production TSX, allow raw controls only in `ImeTextControl.tsx`, and report stable relative paths/lines.

- [ ] **Step 4: Wire lint and prove the tree passes**

Run `npm --prefix apps/desktop run lint`; require ESLint and the IME audit to pass.

- [ ] **Step 5: Amend canon**

Add the approved durable rule, detailed exemptions/tests, architecture boundary, and focused commands. Bump amendment dates.

- [ ] **Step 6: Commit**

Commit as `chore: enforce shared IME-safe text input rules`.

### Task 6: Verify, review, and publish

**Files:** Review all changes against `origin/main`.

- [ ] **Step 1: Run focused gates**

```bash
node --test scripts/check-ime-text-inputs.test.mjs
npm --prefix apps/desktop test -- src/domain/compositionLifecycle.test.ts src/domain/latestAsyncResult.test.ts src/components/ImeTextControl.test.tsx src/components/dialogs.test.tsx src/components/composer.test.tsx src/components/TimelineView.test.tsx
```

- [ ] **Step 2: Run full frontend gates**

```bash
npm --prefix apps/desktop test
npm --prefix apps/desktop run lint
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run build
```

- [ ] **Step 3: Inspect diff**

```bash
git diff --check origin/main...HEAD
git status --short
git diff --stat origin/main...HEAD
```

- [ ] **Step 4: Request independent review and address verified findings**

Review against #272, the design, repository rules, privacy constraints, and the raw-control inventory.

- [ ] **Step 5: Push and create draft PR**

Push `codex/issue-272-ime-text-inputs` and create a draft PR titled `fix: unify IME-safe desktop text inputs` with `Closes #272`, architecture summary, migration inventory, rule changes, and test evidence.
