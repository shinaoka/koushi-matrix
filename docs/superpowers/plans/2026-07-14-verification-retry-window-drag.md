# Verification Retry and Window Drag Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Clear completed verification failures when a retry starts and restore mouse window dragging on every session-verification screen.

**Architecture:** Keep failure lifecycle ownership in the Rust session reducer so every projection agrees on the active attempt. Add a fixed renderer drag strip above the centered verification controls, using the existing guarded Tauri `startDragging()` integration through an injectable callback for deterministic browser tests. Record the attended signed-DMG release procedure in the repository's operational agent notes.

**Tech Stack:** Rust, `koushi-state`, React, TypeScript, Tauri 2, Vitest, Testing Library, CSS.

## Global Constraints

- A completed failure remains visible in `AwaitingVerification` until the user starts a new attempt.
- Starting a supported verification method clears only the previous `VerificationGateState.failure`; gate capabilities, account kind, method validation, flow correlation, and fail-closed admission remain unchanged.
- The verification drag region is 44 pixels high and present for every state rendered by `SessionVerificationGate`.
- Dragging starts only from a primary-button press on the dedicated region and never from verification buttons or inputs.
- Signed-DMG instructions contain no identity, Apple ID, team ID, or app-specific password value.
- No new dependency is introduced.

---

### Task 1: Reset failure state at verification retry admission

**Files:**
- Modify: `crates/koushi-state/tests/session_state.rs`
- Modify: `crates/koushi-state/src/reducer/session.rs`

**Interfaces:**
- Consumes: `AppAction::VerificationMethodSubmitted { method, flow_id }` and an `AwaitingVerification` state whose gate may contain a completed failure.
- Produces: `SessionState::Verifying` with the accepted method and flow ID, the same capabilities and account kind, and `gate.failure == None`.

- [ ] **Step 1: Write the failing reducer test**

Add a focused test to `crates/koushi-state/tests/session_state.rs`:

```rust
#[test]
fn verification_retry_clears_the_completed_attempt_failure() {
    let info = session_info();
    let gate = VerificationGateState {
        methods: vec![VerificationMethodCapability::ExistingDeviceSas],
        account_kind: VerificationAccountKind::ExistingIdentity,
        failure: Some(VerificationGateFailureKind::Timeout),
    };
    let mut state = AppState {
        session: SessionState::AwaitingVerification { info: info.clone(), gate },
        ..AppState::default()
    };

    reduce(
        &mut state,
        AppAction::VerificationMethodSubmitted {
            method: VerificationMethod::ExistingDeviceSas,
            flow_id: 78,
        },
    );

    assert_eq!(
        state.session,
        SessionState::Verifying {
            info,
            gate: VerificationGateState {
                methods: vec![VerificationMethodCapability::ExistingDeviceSas],
                account_kind: VerificationAccountKind::ExistingIdentity,
                failure: None,
            },
            method: VerificationMethod::ExistingDeviceSas,
            flow_id: 78,
            sas_emojis: Vec::new(),
        }
    );
}
```

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```bash
cargo test -p koushi-state verification_retry_clears_the_completed_attempt_failure -- --exact
```

Expected: FAIL because the actual `Verifying.gate.failure` is `Some(Timeout)`.

- [ ] **Step 3: Clear the cloned gate failure at the accepted transition**

In `handle_verification_method_submitted`, make the cloned gate mutable and reset the attempt-scoped field before constructing `SessionState::Verifying`:

```rust
let info = info.clone();
let mut gate = gate.clone();
gate.failure = None;
state.session = SessionState::Verifying {
    info,
    gate,
    method,
    flow_id,
    sas_emojis: Vec::new(),
};
```

- [ ] **Step 4: Run focused and complete state tests and verify GREEN**

Run:

```bash
cargo test -p koushi-state verification_retry_clears_the_completed_attempt_failure -- --exact
cargo test -p koushi-state
```

Expected: the focused test and all `koushi-state` tests PASS.

- [ ] **Step 5: Commit the state fix**

```bash
git add crates/koushi-state/src/reducer/session.rs crates/koushi-state/tests/session_state.rs
git commit -m "Fix stale verification failure on retry"
```

---

### Task 2: Add a dedicated verification-window drag strip

**Files:**
- Modify: `apps/desktop/src/SessionVerificationGate.test.tsx`
- Modify: `apps/desktop/src/App.tsx`
- Modify: `apps/desktop/src/styles.css`

**Interfaces:**
- Consumes: optional `onStartWindowDrag?: () => void` component callback; production default uses the current Tauri window.
- Produces: `.session-verification-drag-region[data-tauri-drag-region]`, invoking the callback only for a primary-button press while leaving the existing verification controls unchanged.

- [ ] **Step 1: Write failing renderer tests**

Add a test to `apps/desktop/src/SessionVerificationGate.test.tsx` that renders an awaiting-verification snapshot with an injected spy:

```tsx
test("provides a primary-button-only verification window drag region", async () => {
  const snapshot = await createBrowserFakeApi({ session: "needsRecovery" }).getSnapshot();
  snapshot.state.domain.session = {
    kind: "awaitingVerification",
    user_id: "@u:example.invalid",
    homeserver: "https://example.invalid",
    device_id: "D",
    gate: { methods: ["existingDeviceSas"], account_kind: "existingIdentity" },
  };
  const onStartWindowDrag = vi.fn();
  const { container } = render(
    <SessionVerificationGate
      snapshot={snapshot}
      onSnapshot={() => undefined}
      onSignOut={() => undefined}
      onStartWindowDrag={onStartWindowDrag}
      operations={{
        startOwnUserSas: async () => snapshot,
        submitRecovery: async () => snapshot,
        retryCurrentDeviceTrustDiscovery: async () => snapshot,
      }}
    />
  );

  const dragRegion = container.querySelector(".session-verification-drag-region");
  expect(dragRegion?.getAttribute("data-tauri-drag-region")).toBe("");
  fireEvent.mouseDown(dragRegion!, { button: 2, buttons: 2 });
  expect(onStartWindowDrag).not.toHaveBeenCalled();
  fireEvent.mouseDown(dragRegion!, { button: 0, buttons: 1 });
  expect(onStartWindowDrag).toHaveBeenCalledTimes(1);
});
```

- [ ] **Step 2: Run the focused UI test and verify RED**

Run:

```bash
npm --prefix apps/desktop test -- --run src/SessionVerificationGate.test.tsx
```

Expected: FAIL because `SessionVerificationGate` has no `onStartWindowDrag` property or drag-region element.

- [ ] **Step 3: Add the guarded production drag operation and component region**

In `apps/desktop/src/App.tsx`, add a default operation near `SessionVerificationGate`:

```tsx
function startSessionVerificationWindowDrag(): void {
  if (!isTauriRuntime()) return;
  void getCurrentWindow().startDragging().catch(() => undefined);
}
```

Extend the component props with `onStartWindowDrag?: () => void`, default it to `startSessionVerificationWindowDrag`, and insert this region as the first child of the existing `<main>` before its `<h1>`:

```tsx
return <main className="session-verification-gate" aria-label={heading}>
  <div
    className="session-verification-drag-region"
    data-tauri-drag-region=""
    aria-hidden="true"
    onMouseDown={(event) => {
      if (event.buttons !== 1) return;
      event.preventDefault();
      onStartWindowDrag();
    }}
  />
  <h1>{heading}</h1>
  {checking && <p>{t("gate.checking")}</p>}
  {discovering && <p>{t("gate.discovering")}</p>}
  {session.kind === "rejecting" && <p>{t("gate.rejecting")}</p>}
  {session.kind === "locked" && <p>{t("gate.locked")}</p>}
  {session.gate?.failureKind && <p role="alert">{gateFailureLabel(session.gate.failureKind)}</p>}
  {preparationFailure && <p role="alert">{gateFailureLabel(preparationFailure)}</p>}
  {operationError && !session.gate?.failureKind && !preparationFailure && <p role="alert">{operationError}</p>}
  {session.kind === "rejecting" && session.reason && <p role="alert">{gateRejectLabel(session.reason)}</p>}
  {awaiting && methods.includes("existingDeviceSas") && <button disabled={gateOperation === "sas"} onClick={() => void run("sas", operations.startOwnUserSas)}>{t("gate.otherDevice")}</button>}
  {sasVerifying && session.sas_emojis?.length === 7 && <div className="session-verification-emojis">{session.sas_emojis.map((emoji, index) => <span key={index}>{emoji.symbol} {emoji.description}</span>)}</div>}
  {sasVerifying && session.sas_emojis?.length === 7 && flowId !== undefined && <><button onClick={() => void run("sas", () => api.confirmSasVerification(flowId))}>{t("gate.match")}</button><button onClick={() => void run("sas", () => api.mismatchSasVerification(flowId))}>{t("gate.mismatch")}</button></>}
  {awaiting && (methods.includes("recoveryKey") || methods.includes("securityPhrase")) && <form onSubmit={(event) => { event.preventDefault(); const secret = recoveryRef.current?.value.trim() ?? ""; if (secret) void run("recovery", () => operations.submitRecovery(secret)); if (recoveryRef.current) recoveryRef.current.value = ""; }}><input ref={recoveryRef} type="password" aria-label={t("gate.recoverySecret")} autoComplete="off"/><button disabled={gateOperation === "recovery"} type="submit">{t("gate.recover")}</button></form>}
  {awaiting && methods.includes("bootstrap") && <form onSubmit={(event) => { event.preventDefault(); const destination = destinationRef.current?.value.trim() ?? ""; const passphrase = passphraseRef.current?.value || null; if (destinationRef.current) destinationRef.current.value = ""; if (passphraseRef.current) passphraseRef.current.value = ""; if (destination) void run("recovery", () => api.startSessionBootstrap(passphrase, destination)); }}><input ref={destinationRef} aria-label={t("gate.destination")}/><input ref={passphraseRef} type="password" aria-label={t("gate.passphrase")} autoComplete="new-password"/><button type="submit">{t("gate.bootstrap")}</button></form>}
  {session.kind === "awaitingBootstrapConfirmation" && flowId !== undefined && <button onClick={() => void run("recovery", () => api.confirmSessionBootstrapSaved(flowId))}>{t("gate.saved")}</button>}
  {(awaiting || discovering || rechecking) && <button disabled={gateOperation === "recovery"} onClick={() => void run("recovery", operations.retryCurrentDeviceTrustDiscovery)}>{t("gate.retry")}</button>}
  {sasVerifying && flowId !== undefined && <button onClick={() => void run("sas", () => api.cancelVerification(flowId))}>{t("action.cancel")}</button>}
  <button onClick={onSignOut}>{t("gate.signOut")}</button>
</main>;
```

- [ ] **Step 4: Move centering styles to the content and size the drag strip**

Update `apps/desktop/src/styles.css` without changing the existing centered gate layout:

```css
.session-verification-gate {
  min-height: 100vh;
  display: grid;
  place-content: center;
  gap: 12px;
  max-width: 560px;
  margin: 0 auto;
  padding: 32px;
}

.session-verification-drag-region {
  position: fixed;
  inset: 0 0 auto;
  height: 44px;
}
```

- [ ] **Step 5: Run focused UI tests and static checks and verify GREEN**

Run:

```bash
npm --prefix apps/desktop test -- --run src/SessionVerificationGate.test.tsx
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run lint
```

Expected: all commands PASS.

- [ ] **Step 6: Commit the renderer fix**

```bash
git add apps/desktop/src/App.tsx apps/desktop/src/SessionVerificationGate.test.tsx apps/desktop/src/styles.css
git commit -m "Restore verification window dragging"
```

---

### Task 3: Document the signed macOS DMG procedure

**Files:**
- Modify: `AGENTS.md`

**Interfaces:**
- Consumes: `npm --prefix apps/desktop run build:dmg`, the existing Tauri macOS signing configuration, a locally installed Developer ID Application certificate, and Apple notarization environment variables.
- Produces: a secret-free attended release recipe covering signing identity discovery, build/notarization, artifact verification, and opening the DMG.

- [ ] **Step 1: Add the operational procedure**

Add a `## Signed macOS DMG` section to `AGENTS.md` with these commands and explanations:

```bash
security find-identity -v -p codesigning
read -r "APPLE_SIGNING_IDENTITY?Developer ID Application identity: "
read -r "APPLE_ID?Apple ID: "
read -r "APPLE_TEAM_ID?Apple Team ID: "
read -rs "APPLE_PASSWORD?App-specific password: "; echo
export APPLE_SIGNING_IDENTITY APPLE_ID APPLE_TEAM_ID APPLE_PASSWORD

npm --prefix apps/desktop run release:preflight
npm --prefix apps/desktop run build:dmg

APP="$(find apps/desktop/src-tauri/target/release/bundle/macos -maxdepth 1 -name '*.app' -print -quit)"
DMG="$(find apps/desktop/src-tauri/target/release/bundle/dmg -maxdepth 1 -name '*.dmg' -print -quit)"
test -n "$APP" && test -n "$DMG"
codesign --verify --deep --strict --verbose=2 "$APP"
xcrun stapler validate "$APP"
spctl --assess --type execute --verbose=4 "$APP"
codesign --verify --verbose=2 "$DMG"
xcrun stapler validate "$DMG"
spctl --assess --type open --context context:primary-signature --verbose=4 "$DMG"
open "$DMG"
```

State that the `APPLE_PASSWORD` value is an app-specific password, all four variables are session-only secrets that must never be committed or pasted into logs, the build command signs and submits for notarization through Tauri, and a successful command is not sufficient until `codesign`, `stapler`, and `spctl` all accept the artifacts.

- [ ] **Step 2: Check the documentation for secrets and command drift**

Run:

```bash
git diff --check -- AGENTS.md
npm --prefix apps/desktop run build:dmg -- --help
npm --prefix apps/desktop run release:preflight
```

Expected: whitespace check passes, the documented build entry point is printed, and release configuration preflight passes without requiring secret values.

- [ ] **Step 3: Commit the release instructions**

```bash
git add AGENTS.md
git commit -m "Document signed macOS DMG workflow"
```

---

### Task 4: Verify, publish, and integrate

**Files:**
- Verify only: all files changed by Tasks 1 and 2.

**Interfaces:**
- Consumes: the two independently committed fixes.
- Produces: a reviewed GitHub PR merged into `main` only after required checks pass.

- [ ] **Step 1: Run the complete proportional verification suite**

```bash
cargo fmt --check
cargo test -p koushi-state
npm --prefix apps/desktop test -- --run src/SessionVerificationGate.test.tsx src/App.test.tsx src/components/Shell.test.tsx
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run lint
git diff --check origin/main...HEAD
```

Expected: every command exits zero and `git diff --check` reports no errors.

- [ ] **Step 2: Review the final branch scope**

```bash
git status --short --branch
git diff --stat origin/main...HEAD
git log --oneline origin/main..HEAD
```

Expected: only the spec, plan, `AGENTS.md`, reducer/test, and verification renderer/test/style changes are committed; the pre-existing untracked user files remain unmodified.

- [ ] **Step 3: Push and create a ready PR**

```bash
git push -u origin codex/verification-retry-window-drag
gh pr create --base main --head codex/verification-retry-window-drag --title "Fix verification retry state and window dragging" --body-file /tmp/verification-retry-window-drag-pr.md
```

Expected: GitHub returns the new PR URL.

- [ ] **Step 4: Wait for required checks and merge**

```bash
gh pr checks --watch <PR_NUMBER>
gh pr merge <PR_NUMBER> --merge --delete-branch
```

Expected: all required checks PASS and GitHub reports the PR merged.

- [ ] **Step 5: Synchronize local `main` without touching user files**

```bash
git switch main
git pull --ff-only origin main
git status --short --branch
```

Expected: `main` matches `origin/main`; the three pre-existing untracked paths remain present and unchanged.
