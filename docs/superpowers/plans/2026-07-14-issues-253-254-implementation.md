# Issues #253 and #254 Implementation Plan

> Implement from the approved batch design in
> `docs/superpowers/specs/2026-07-14-issues-253-254-design.md`. Preserve the
> Rust-owned session/trust and formatted-HTML safety boundaries. Use strict
> RED-GREEN-REFACTOR ordering and deliver one non-squash PR closing both issues.

**Goal:** Make verified warm session restore network-independent with a single
classic-sync owner, and render pretty-printed formatted lists without synthetic
line breaks.

**Architecture:** AccountActor retains the authoritative SDK trust subscriber,
but hands classic-sync ownership from an optional provisional restricted lane
to the normal SyncActor through a stop/join and projection-ack barrier. React
stops treating normal sync health as a verification state. Formatted HTML keeps
the existing Rust-sanitized DTO and UTF-16 link basis; React only adapts source
whitespace into normal HTML flow at render time.

**Technology:** Rust/Tokio actors, matrix-rust-sdk, React/TypeScript, Vitest,
Testing Library, Playwright, CSS contract tests, local Conduit/Tuwunel QA.

## Task 1: RED tests for verified restore ownership

**Files:**

- Modify: `crates/koushi-core/src/account.rs`
- Modify: `crates/koushi-core/src/bin/headless-core-qa.rs`

1. Extend test-only AccountActor runtime inspection to report restricted,
   promotion, and normal sync owners independently.
2. Add a test in `account.rs` proving an initial authoritative `Verified`
   observation reaches the Ready projection without completing a promotion
   full-state override and without starting restricted sync.
3. Add a test proving a provisional restricted lane is cancelled and joined
   before Ready is dispatched and normal sync starts only after projection ACK.
4. Add/extend restore tests for `Unknown`, `Unverified`, offline verified
   restore, and Ready-to-Locked downgrade.
5. Extend admission-diagnostic tests to require ordered store/trust/catch-up/
   Ready/normal-owner phases and private-data-free fields.
6. Extend `gate_restore` with bounded Ready evidence that does not expose
   Matrix identifiers or raw errors.
7. Run the focused tests and record the expected failures:

   ```bash
   cargo test -p koushi-core --lib verified_warm_restore
   cargo test -p koushi-core --lib verification_to_normal_sync_handoff
   cargo test -p koushi-core --lib warm_restore_diagnostics
   ```

## Task 2: GREEN verified restore and exclusive handoff

**Files:**

- Modify: `crates/koushi-core/src/account.rs`
- Modify: `crates/koushi-sdk/src/lib.rs`
- Modify: `crates/koushi-sdk/tests/password_login.rs`
- Modify: `docs/architecture/overview.md`
- Modify: `docs/architecture/state-machine.md`
- Modify: `docs/policies/engineering-rules.md`

1. Change provisional startup to subscribe/read trust first and skip restricted
   sync for an initial `Verified` value.
2. Replace promotion full-state task state with a generation-safe handoff that
   cancels and joins any restricted task before dispatching authoritative
   `Verified`.
3. Preserve the trust observer after Ready; remove the promoted restricted
   lane and start normal SyncActor only after the AppActor projection ACK.
4. Remove `PromotionFullStateSyncFinished`, its override/test plumbing, the SDK
   full-state settings/helper, and obsolete tests.
5. Emit coarse timed admission diagnostics for trust read, catch-up skipped or
   stopped, Ready dispatch, and normal sync start.
6. Update architecture/state-machine/engineering canon with the single-owner
   sequence and the distinction between trust readiness and sync health.
7. Run:

   ```bash
   cargo fmt --all -- --check
   cargo test -p koushi-core --lib verified_warm_restore
   cargo test -p koushi-core --lib verification_to_normal_sync_handoff
   cargo test -p koushi-core --lib warm_restore_diagnostics
   cargo test -p koushi-core --lib trust_
   cargo test -p koushi-state --test session_state
   cargo test -p koushi-sdk --lib restricted_verification
   cargo test -p koushi-sdk --test password_login
   ```

8. Commit the Rust/canon slice.

## Task 3: RED and GREEN session gate UI

**Files:**

- Modify: `apps/desktop/src/App.tsx`
- Modify: `apps/desktop/src/SessionVerificationGate.test.tsx`
- Modify: `apps/desktop/src/App.test.tsx`
- Modify: `apps/desktop/e2e/session-verification-gate.spec.ts`

1. Add unit tests requiring `CheckingTrust` to use the checking text for both
   the main landmark label and heading.
2. Add unit/browser tests requiring Ready plus Starting, Reconnecting, and
   Failed sync states to render the normal shell, never the verification gate;
   require the existing restart action for Failed.
3. Run the focused tests and confirm the new cases fail.
4. Use the phase-specific checking label in `SessionVerificationGate` and
   remove `Ready && sync !== running` from the verification-gate condition.
5. Run:

   ```bash
   npm --prefix apps/desktop run test -- --run src/SessionVerificationGate.test.tsx src/App.test.tsx
   npm --prefix apps/desktop run typecheck
   npm --prefix apps/desktop exec -- playwright test e2e/session-verification-gate.spec.ts --workers=1
   ```

6. Commit the session UI slice.

## Task 4: RED formatted whitespace tests

**Files:**

- Modify: `apps/desktop/src/components/TimelineView.test.tsx`
- Modify: `apps/desktop/src/styles.contract.test.ts`
- Modify: `apps/desktop/e2e/basic-operations.spec.ts`

1. Add component fixtures for pretty/minified paragraph-list-paragraph HTML,
   nested lists, inline strong/emphasis spacing, explicit `br`, exact pre/code,
   and CJK lists.
2. Assert ordinary source newlines create no `br`, explicit `br` creates one,
   and every list element child is `li`.
3. Add a style contract requiring the formatted-body root not to use grid.
4. Add a browser check comparing pretty/minified DOM semantics and rendered
   heights within a small tolerance.
5. Run the focused unit and browser cases and confirm the expected failures.

## Task 5: GREEN formatted whitespace and normal flow

**Files:**

- Modify: `apps/desktop/src/components/TimelineView.tsx`
- Modify: `apps/desktop/src/styles.css`

1. Keep tokenization and linkification unchanged so Rust UTF-16 ranges retain
   their current input basis.
2. Render ordinary text directly through query highlighting; do not split on
   newline or synthesize `br` nodes.
3. Filter contextual whitespace only while rendering children: omit
   whitespace-only direct list children and block-boundary formatting text,
   preserving meaningful inline sibling spacing.
4. Keep the explicit sanitized `br` renderer and current pre/code path.
5. Change the formatted-body root from grid to normal flow and use adjacent
   block/list rules for compact nested spacing.
6. Run:

   ```bash
   npm --prefix apps/desktop run test -- --run src/components/TimelineView.test.tsx src/styles.contract.test.ts
   npm --prefix apps/desktop run typecheck
   npm --prefix apps/desktop exec -- playwright test e2e/basic-operations.spec.ts -g "formatted.*list|pretty.*minified" --workers=1
   ```

7. Commit the formatted-message slice.

## Task 6: Integrated verification and independent review

**Files:** all changed files.

1. Run formatting and diff hygiene:

   ```bash
   cargo fmt --all -- --check
   npm --prefix apps/desktop run lint
   git diff --check origin/main...HEAD
   ```

2. Run all focused issue gates again, then:

   ```bash
   cargo test -p koushi-core --lib
   cargo test -p koushi-state --test session_state
   cargo test -p koushi-sdk --lib
   npm --prefix apps/desktop run test
   npm --prefix apps/desktop run typecheck
   ```

3. Run local integration evidence:

   ```bash
   PATH=/tmp/koushi-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:headless-local -- --server=conduit --scenario=gate_restore --core --core-backend=both --timeout-ms=240000
   ```

4. Generate a complete diff including new files and run `codex review -`
   against repository canon and both issue acceptance criteria. Verify every
   finding before adopting or rejecting it.
5. Re-run affected gates after any review fix and commit the final changes.

## Task 7: PR, CI, and non-squash merge

1. Rebase or merge the current `origin/main` only if required, rerunning all
   affected checks after conflict resolution.
2. Push the branch and create one PR whose body summarizes both fixes, lists
   verification evidence, and contains `Closes #253` and `Closes #254`.
3. Monitor required GitHub checks to completion. Diagnose and fix failures from
   logs; do not merge with required checks pending or failing.
4. Merge with a merge commit (not squash), verify the PR is `MERGED`, verify
   both issues are closed, and verify the merge commit is reachable from
   `origin/main`.
