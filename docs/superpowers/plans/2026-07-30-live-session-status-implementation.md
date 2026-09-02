# Live Session Status and Device Naming Implementation Plan

> Follow `docs/superpowers/specs/2026-07-30-live-session-status-design.md`.
> Every behavior task starts with a failing headless assertion.

**Issue:** #369

> #802 amendment: the original fail-closed replacement assertions below apply
> to the current inspection verdict, not destruction of prior observational
> facts. `Checking`/`Failed` now retain `last_known_details` across transient
> connectivity failure, and core-owned `Recovery` refreshes it after proven sync
> recovery. See `2026-09-02-issue-802-degraded-network-recovery.md`.

**Goal:** Conditionally name empty OAuth devices and expose one authoritative
Rust-owned current-session status through an accessible top-bar popover.

## Task 1: Establish the state-machine contract

**Files**

- Create `crates/koushi-state/src/state/session_status.rs`
- Modify `crates/koushi-state/src/state/mod.rs`
- Modify `crates/koushi-state/src/action.rs`
- Modify `crates/koushi-state/src/effect.rs`
- Modify `crates/koushi-state/src/reducer.rs`
- Create `crates/koushi-state/src/reducer/session_status.rs`
- Create `crates/koushi-state/tests/session_status_state.rs`
- Modify `docs/architecture/state-machine.md`

**RED**

Add reducer tests for:

- `Idle -> Checking` from open/manual triggers;
- duplicate refresh rejection;
- correlated `Checking -> Ready`;
- failed refresh replacing prior ready data with `Failed`;
- stale request completion rejection;
- logout/account-clear reset;
- `Verified` requiring both owner-cross-signed current device and verified own
  identity;
- key-backup and sync state remaining explicit fields rather than altering the
  verification verdict.

Run:

```bash
cargo test -p koushi-state --test session_status_state
```

**GREEN**

Implement the types, actions, effects, reducer guards, and AppState field.
Keep identifiers out of custom `Debug` output.

## Task 2: Make authentication method durable

**Files**

- Modify `crates/koushi-state/src/state/session.rs`
- Modify state fixtures constructing `SessionInfo`
- Modify `crates/koushi-sdk/src/lib.rs`
- Modify `crates/koushi-core/src/account.rs`
- Modify persistence compatibility tests

**RED**

Add tests proving:

- password, SSO, OAuth, and token completion record the right coarse method;
- legacy persisted `SessionInfo` without the field restores as `Unknown`;
- serialization contains no authentication credentials.

Run focused state/SDK/core tests with `--lib` where applicable.

**GREEN**

Add `SessionAuthenticationMethod` with a serde default and populate it at each
known login-completion boundary.

## Task 3: Add SDK-backed session inspection

**Files**

- Modify `crates/koushi-sdk/src/lib.rs`
- Modify the vendored SDK only if the public SDK surface cannot expose a
  required authoritative fact; keep any test hook behind `testing`

**RED**

Using `MatrixMockServer`, test:

- current device found with display name;
- device absent;
- current device cross-signed by owner;
- own identity verified/unverified;
- key backup ready/disabled/unknown;
- device or identity request failure mapped to a coarse error;
- `Debug` and serialization omit raw errors and secrets.

Run:

```bash
cargo test -p koushi-sdk --lib current_session_status
```

**GREEN**

Add a single SDK session-inspection result and error type. Query the current
device and own identity in one method and return coarse facts only.

## Task 4: Add conditional OAuth device naming

**Files**

- Modify `crates/koushi-state/src/locale_profile.rs`
- Modify `crates/koushi-sdk/src/lib.rs`
- Modify `crates/koushi-core/src/account.rs`
- Modify focused SDK/core tests

**RED**

Prove:

- each `DisplayPlatform` maps to the required `Koushi on …` label;
- empty or whitespace-only OAuth device names produce one rename;
- non-empty names produce no rename request;
- rename failure does not fail login or stop sync;
- restore/restart does not rename an already named device;
- diagnostics expose only coarse outcomes.

**GREEN**

Pass the Rust-owned display platform into the OAuth completion command/actor
boundary, inspect the authoritative device once, and conditionally rename.
Do not add launch-time retry machinery.

## Task 5: Route current-session refresh through core

**Files**

- Modify `crates/koushi-core/src/command.rs`
- Modify `crates/koushi-core/src/account.rs`
- Modify `crates/koushi-core/src/runtime.rs`
- Modify `crates/koushi-core/src/event.rs` if the existing state delta requires
  a new changed-slice marker
- Modify focused core tests

**RED**

Prove:

- open/manual commands enter `Checking`;
- both runtime effect lanes reach `AccountActor`;
- actor completion uses request and session-generation fencing;
- SDK failure emits `Failed` and cannot retain stale `Verified`;
- logout aborts pending work and stale completion cannot repopulate state;
- lifecycle diagnostics contain trigger, result, and elapsed time only.

Run:

```bash
cargo test -p koushi-core --lib session_status
cargo test -p koushi-core --lib runtime_routes_current_session_status
```

**GREEN**

Implement the command/effect/actor path, bounded refresh task, state projection,
and private-data-free diagnostics. Reuse #375's production effect routing.

## Task 6: Mirror the transport contract

**Files**

- Modify `apps/desktop/src-tauri/src/dto.rs`
- Modify `apps/desktop/src-tauri/src/commands/mod.rs`
- Modify `apps/desktop/src/domain/types.ts`
- Modify `apps/desktop/src/backend/DesktopApi.ts`
- Modify `apps/desktop/src/backend/tauriApi.ts`
- Modify `apps/desktop/src/backend/browserFakeApi.ts`
- Modify app-harness/Tauri IPC mock snapshots
- Modify `apps/desktop/src-tauri/tests/golden/frontend_app_state.json`
- Modify `apps/desktop/src/domain/coreEvents.generated.json` if delta shape changes

**RED**

Add DTO serialization and TypeScript contract tests before fields/commands.

**GREEN**

Mirror the full state and command without React-local defaults. Populate the
golden with a non-empty maximally informative `Ready` value.

Run:

```bash
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib frontend_app_state_golden
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml core_event_wire_format_matches_checked_in_contract_artifact
npm --prefix apps/desktop run typecheck
```

## Task 7: Build the accessible popover

**Files**

- Modify `apps/desktop/src/components/Shell.tsx`
- Modify `apps/desktop/src/components/Shell.test.tsx`
- Modify `apps/desktop/src/App.tsx`
- Modify `apps/desktop/src/styles.css`
- Modify `apps/desktop/src/i18n/messages.ts`
- Modify `apps/desktop/src/i18n/messages.test.ts`
- Modify the relevant Playwright spec

**RED**

Add browser-headless tests for:

- pointer and keyboard opening;
- immediate `Checking` command on open;
- all required fields from Rust-shaped state;
- manual recheck and delayed Checking;
- error replacing stale verified display and retry;
- Device ID-only copy;
- discovered external management URL;
- local settings fallback when no safe URL exists;
- diagnostics link;
- Escape/outside-click dismissal and focus return.

**GREEN**

Turn the sync-status surface into a button, add a compact popover, and render
only the Rust/Tauri DTO. Preserve `aria-live` sync announcements without making
the whole button a mutable live region.

Run:

```bash
npm --prefix apps/desktop run test -- --run src/components/Shell.test.tsx
npm --prefix apps/desktop run test -- --run src/i18n/messages.test.ts
npm --prefix apps/desktop exec -- playwright test e2e/session-status.spec.ts --workers=1
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run lint
```

## Task 8: Add local homeserver acceptance proof

**Files**

- Modify `crates/koushi-core/src/bin/headless-core-qa.rs`
- Modify local QA scenario parsing/tests

**RED**

Add a `session_status` scenario that fails until it can:

- log in a current device;
- prove an empty OAuth-style device name is repaired once where the harness can
  model the flow;
- request current-session status;
- observe Checking then Ready from SDK-backed data;
- prove the server-side device remains the same device with the expected name;
- emit only private-data-free success tokens.

**GREEN**

Implement the shortest deterministic scenario against local Conduit/Tuwunel.
If the local server cannot exercise MAS/OAuth, keep the actual OAuth rename
proof in the SDK mock transport and use the homeserver lane for authoritative
current-device/session-status settlement.

Run the exact repository QA entry point once after focused gates are green.

## Task 9: Full verification and self-review

Run, reading each command's own exit status:

```bash
node scripts/check-sdk-submodule.mjs
cargo fmt --all -- --check
cargo test -p koushi-state --test session_status_state
cargo test -p koushi-sdk --lib
cargo test -p koushi-core --lib
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run lint
npm --prefix apps/desktop test
npm --prefix apps/desktop exec -- playwright test e2e/session-status.spec.ts --workers=1
cargo test --workspace
git diff --check origin/main...HEAD
```

Then run the integrated local QA scenario once, review:

```bash
git diff origin/main...HEAD
git status --short
```

Check the diff against `REPOSITORY_RULES.md`, architecture/state-machine canon,
engineering rules, this plan, privacy constraints, DTO completeness, and all
untracked files. Request review, address findings, push, open one standalone
PR with `Closes #369`, wait for required CI, and merge with a merge commit
(never squash).
