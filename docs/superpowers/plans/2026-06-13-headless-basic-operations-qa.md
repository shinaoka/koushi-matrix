# Headless Basic Operations QA Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Verify basic Matrix desktop operations first without GUI, then prove the same core workflows through a real Linux Tauri client running on a virtual display.

**Architecture:** Headless QA is the contract layer: Conduit and Tuwunel, probed SyncService plus forced LegacySync, two synthetic users, all cleanup local. matrix.org QA is the slow compatibility lane: one login per run, file credential store only, unique synthetic rooms/spaces, aggressive cleanup, bounded retries, and no repeated destructive cycles. Linux virtual-display QA is the product-integration lane: launch the actual Tauri client under Xvfb/tauri-driver, drive the UI with WebDriver, and assert the same operation tokens that headless already proved.

**Tech Stack:** Rust `matrix-desktop-core` QA binaries, vendored `matrix-rust-sdk`/`matrix-sdk-ui`, Node QA runners, Conduit, Tuwunel, matrix.org credentials in `.local-secrets`, existing secret scan and release-gate scripts.

---

## Current Baseline

- `crates/matrix-desktop-core/src/bin/headless-core-qa.rs` already covers local login/sync, room creation, space creation, `SetSpaceChild`, invite/join, room list normalization, send/receive, a plain B response, edit, redact, backward pagination, CJK search, restore, logout, and cleanup.
- `scripts/desktop-headless-local-qa.mjs` already starts disposable local Conduit/Tuwunel servers, registers two synthetic users, and can run both the SDK-level and core-level local QA.
- `crates/matrix-desktop-core/src/bin/real-homeserver-qa.rs` already covers matrix.org login/recovery/sync, synthetic room operations, send/edit/redact/search, restore, leave/forget, logout, and transcript redaction.
- Missing pieces for the requested next phase:
  - staged scenario selection rather than one large all-or-nothing QA flow,
  - explicit token contracts per operation group,
  - true Matrix reply relation (`m.in_reply_to`) separate from "B sends a normal response",
  - thread reply coverage after the reply relation lands,
  - matrix.org space creation/set-child compatibility in the real lane,
  - a clear command set for fast local iteration and slow real-server acceptance.

## Server Strategy

Local lane:

- Run on every Matrix-behavior change.
- Use Conduit and Tuwunel.
- Run both backend legs per server:
  - probed SyncService,
  - forced LegacySync via existing debug/test override.
- Use two fresh synthetic users per run.
- Allow destructive operations because the homeserver data dir is disposable.

matrix.org lane:

- Run manually before accepting a Matrix behavior phase, and before release-confidence claims.
- Use only `.local-secrets/real-account-qa/credentials.json`.
- Current credentials provide one real account; do not assume a second matrix.org account exists.
- Keep one login per run.
- Use unique names/tokens containing timestamp + short random suffix.
- Cleanup must leave/forget synthetic rooms and spaces and logout even after earlier failures.
- If a real-server operation cannot be cleaned up reliably, stop and report the gap instead of expanding the lane.

Linux virtual-display lane:

- Run after the headless local lane is green.
- Use the actual Linux Tauri app, not the browser harness.
- Run under Xvfb with `tauri-driver` and WebDriverIO.
- Start with a disposable local homeserver, not matrix.org, so GUI iteration is safe and fast.
- Use the same file credential store override and QA login transport; no OS keychain prompts.
- Prove that a user can reach a ready synced client and perform basic operations from the client surface.
- matrix.org GUI smoke remains optional/attended until the local virtual-display client flow is stable.

## Scenario Ladder

1. `safety`: file credential store guard, secret redaction, server capability probe.
2. `login_sync`: login, restore gate, sync started/running, backend token.
3. `room_space`: create room, create space, set room as child, invite/join locally, assert room/space list projection.
4. `timeline`: subscribe, send text, local echo, SendCompleted, remote receive, backward pagination.
5. `reply`: add and verify true Matrix reply relation (`m.in_reply_to`), not just a normal response message.
6. `thread`: send root, send reply in `TimelineKind::Thread`, assert room timeline/thread timeline behavior.
7. `edit_redact_search`: edit, redact, CJK search, stale-search rejection after edit/redact.
8. `restore_cleanup`: stop sync, restore encrypted store, resubscribe, leave/forget synthetic artifacts, logout, post-logout restore failure.

The local lane should eventually run all scenarios. The matrix.org lane should start with `safety,login_sync,timeline,edit_redact_search,restore_cleanup`, then add `room_space` after real-server space cleanup is proven, and add `reply/thread` after those commands are implemented and local lanes are green.

The Linux virtual-display lane should start with `login_sync` and `timeline`
through the real client, then add room/space creation, reply, and thread
operations as those controls exist in the product UI.

## Task 1: Define Scenario Token Contract

**Files:**
- Create: `docs/qa/headless-basic-operations.md`
- Modify: `docs/superpowers/plans/2026-06-13-roadmap-phases-10-18.md`

- [x] **Step 1: Write the QA contract doc**

Create `docs/qa/headless-basic-operations.md` with this content:

```markdown
# Headless Basic Operations QA

This QA runs without GUI. It verifies Matrix operations through
CoreCommand/CoreEvent only.

## Local Servers

Run:

```bash
npm --prefix apps/desktop run qa:headless-local -- --server=both --core --scenario=all
```

Required final tokens:

```text
safety=ok
login_sync=ok
room_space=ok
timeline=ok
reply=ok
thread=ok
edit_redact_search=ok
restore_cleanup=ok
```

## matrix.org

Run:

```bash
npm --prefix apps/desktop run qa:real-homeserver -- --scenario=compat
```

The matrix.org lane is rate-limited and uses one login per run. It must not
touch the OS keychain. It must clean up created rooms/spaces and logout even
after an earlier step fails.

Required final tokens for the first compatibility stage:

```text
safety=ok
login_sync=ok
timeline=ok
edit_redact_search=ok
restore_cleanup=ok
```

`room_space`, `reply`, and `thread` become required on matrix.org only after
the local lane proves those scenarios and the real-server cleanup path is
implemented.
```

- [x] **Step 2: Link the contract from the roadmap**

In `docs/superpowers/plans/2026-06-13-roadmap-phases-10-18.md`, add the QA contract under the phase that implements this work:

```markdown
Headless basic-operation QA is tracked in
[headless-basic-operations.md](../../qa/headless-basic-operations.md).
Local lanes run all scenarios; matrix.org runs the compatibility subset until
space cleanup and reply/thread support are proven locally.
```

- [x] **Step 3: Verify docs**

Run:

```bash
git diff --check docs/qa/headless-basic-operations.md docs/superpowers/plans/2026-06-13-roadmap-phases-10-18.md
```

Expected: exit 0.

- [x] **Step 4: Commit**

```bash
git add docs/qa/headless-basic-operations.md docs/superpowers/plans/2026-06-13-roadmap-phases-10-18.md
git commit -m "docs: define headless basic operations QA"
```

## Task 2: Add Scenario Selection To Local Runner

**Files:**
- Modify: `scripts/desktop-headless-local-qa.mjs`
- Modify: `crates/matrix-desktop-core/src/bin/headless-core-qa.rs`
- Test: `apps/desktop/src/scripts/releaseScripts.test.ts`

- [ ] **Step 1: Add failing script tests**

Add tests to `apps/desktop/src/scripts/releaseScripts.test.ts`:

```ts
test("headless local QA forwards scenario selection to core QA", () => {
  const source = readFileSync(
    new URL("../../../../scripts/desktop-headless-local-qa.mjs", import.meta.url),
    "utf8"
  );

  expect(source).toContain("--scenario");
  expect(source).toContain("MATRIX_DESKTOP_QA_SCENARIO");
});

test("headless local QA documents the staged scenario names", () => {
  const output = runScript("scripts/desktop-headless-local-qa.mjs", ["--list"]);

  expect(output).toContain("scenario safety");
  expect(output).toContain("scenario room_space");
  expect(output).toContain("scenario reply");
  expect(output).toContain("scenario thread");
});
```

- [ ] **Step 2: Run test to verify RED**

```bash
npm --prefix apps/desktop run test -- src/scripts/releaseScripts.test.ts
```

Expected: fails because `--scenario` is not implemented.

- [ ] **Step 3: Implement Node option forwarding**

In `scripts/desktop-headless-local-qa.mjs`:

```js
const scenarioOption = optionValue("--scenario") ?? "all";
```

Add these entries to `checks`:

```js
"scenario safety",
"scenario login_sync",
"scenario room_space",
"scenario timeline",
"scenario reply",
"scenario thread",
"scenario edit_redact_search",
"scenario restore_cleanup"
```

When spawning the core QA binary, include:

```js
MATRIX_DESKTOP_QA_SCENARIO: scenarioOption
```

- [ ] **Step 4: Implement Rust scenario parsing**

In `crates/matrix-desktop-core/src/bin/headless-core-qa.rs`, add:

```rust
const ENV_QA_SCENARIO: &str = "MATRIX_DESKTOP_QA_SCENARIO";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QaScenario {
    All,
    Safety,
    LoginSync,
    RoomSpace,
    Timeline,
    Reply,
    Thread,
    EditRedactSearch,
    RestoreCleanup,
}

impl QaScenario {
    fn from_env() -> Result<Self, String> {
        match std::env::var(ENV_QA_SCENARIO).unwrap_or_else(|_| "all".to_owned()).as_str() {
            "all" => Ok(Self::All),
            "safety" => Ok(Self::Safety),
            "login_sync" => Ok(Self::LoginSync),
            "room_space" => Ok(Self::RoomSpace),
            "timeline" => Ok(Self::Timeline),
            "reply" => Ok(Self::Reply),
            "thread" => Ok(Self::Thread),
            "edit_redact_search" => Ok(Self::EditRedactSearch),
            "restore_cleanup" => Ok(Self::RestoreCleanup),
            other => Err(format!("unsupported {ENV_QA_SCENARIO}: {other}")),
        }
    }
}
```

Add `scenario: QaScenario` to `QaConfig` and populate it from `QaScenario::from_env()`.

- [ ] **Step 5: Keep first implementation behavior-preserving**

For this task, do not split the monolithic flow yet. If `scenario != All`, fail with a clear message:

```rust
if config.scenario != QaScenario::All {
    return Err("staged scenarios are parsed but not wired yet".to_owned());
}
```

- [ ] **Step 6: Verify**

```bash
npm --prefix apps/desktop run test -- src/scripts/releaseScripts.test.ts
cargo test -p matrix-desktop-core --lib
```

Expected: both pass.

- [ ] **Step 7: Commit**

```bash
git add scripts/desktop-headless-local-qa.mjs crates/matrix-desktop-core/src/bin/headless-core-qa.rs apps/desktop/src/scripts/releaseScripts.test.ts
git commit -m "qa: add headless local scenario selection"
```

## Task 3: Split Local QA Into Staged Scenario Functions

This task now lands as staged execution in `headless-core-qa.rs` with
scenario-aware early exits and cleanup reuse. `reply` and `thread` stay
explicitly unsupported until the true Matrix reply relation exists.

**Files:**
- Modify: `crates/matrix-desktop-core/src/bin/headless-core-qa.rs`

- [x] **Step 1: Introduce staged scenario helpers**

Add local helpers near `run_async` for scenario gating, implemented final
tokens, final reports, and cleanup reuse:

```rust
enum QaStage { Safety, LoginSync, RoomSpace, Timeline, EditRedactSearch, RestoreCleanup }
fn stages_for_scenario(scenario: QaScenario) -> Vec<QaStage>;
fn final_tokens_for_scenario(scenario: QaScenario) -> Vec<&'static str>;
async fn cleanup_after_login_sync(...) -> Result<String, String>;
async fn cleanup_after_full_flow(...) -> Result<String, String>;
```

- [x] **Step 2: Keep behavior and add scenario-aware early exits**

Use the existing helper functions and exact event waits. Do not change
operation semantics in this task. Supported scenarios run through the
requested stage, then perform restore/logout cleanup where possible.

- [x] **Step 3: Emit one final token per implemented stage**

After each implemented stage completes, print only:

```rust
println!("safety=ok");
println!("login_sync=ok");
println!("room_space=ok");
println!("timeline=ok");
println!("edit_redact_search=ok");
println!("restore_cleanup=ok");
```

Do not emit `reply=ok` or `thread=ok` in Task 3. Every supported scenario
includes `safety=ok` in its final token report because the guard/preflight
is a completed prerequisite for the run.

- [x] **Step 4: Verify local lane**

```bash
npm --prefix apps/desktop run qa:headless-local -- --server=both --core --scenario=all
```

Expected: all four local legs pass and include the stage tokens.

- [ ] **Step 5: Commit**

```bash
git add crates/matrix-desktop-core/src/bin/headless-core-qa.rs
git commit -m "qa: split headless local operations into stages"
```

## Task 4: Add True Matrix Reply Command

**Files:**
- Modify: `crates/matrix-desktop-core/src/command.rs`
- Modify: `crates/matrix-desktop-core/src/timeline.rs`
- Modify: `crates/matrix-desktop-core/src/bin/headless-core-qa.rs`
- Test: `crates/matrix-desktop-core/src/timeline.rs`

- [ ] **Step 1: Add failing command redaction test**

In `crates/matrix-desktop-core/src/timeline.rs` tests, add:

```rust
#[test]
fn send_reply_debug_redacts_body_and_event_ids() {
    let command = TimelineCommand::SendReply {
        request_id: RequestId(7),
        key: TimelineKey::room(AccountKey("@a:test".to_owned()), "!room:test".to_owned()),
        transaction_id: "txn-reply".to_owned(),
        in_reply_to_event_id: "$event:test".to_owned(),
        body: "secret reply body".to_owned(),
    };

    let debug = format!("{command:?}");
    assert!(!debug.contains("secret reply body"));
    assert!(!debug.contains("$event:test"));
    assert!(debug.contains("SendReply"));
}
```

- [ ] **Step 2: Run test to verify RED**

```bash
cargo test -p matrix-desktop-core send_reply_debug_redacts_body_and_event_ids
```

Expected: fails because `SendReply` does not exist.

- [x] **Step 3: Add `TimelineCommand::SendReply`**

In `crates/matrix-desktop-core/src/command.rs`:

```rust
SendReply {
    request_id: RequestId,
    key: TimelineKey,
    transaction_id: String,
    in_reply_to_event_id: String,
    body: String,
},
```

Update `CoreCommand::request_id()` and the custom `Debug` impl so `body` and `in_reply_to_event_id` are redacted.

- [x] **Step 4: Route the command through TimelineActor**

In `crates/matrix-desktop-core/src/timeline.rs`, add a `TimelineActorMessage::SendReply` variant:

```rust
SendReply {
    request_id: RequestId,
    key: TimelineKey,
    transaction_id: String,
    in_reply_to_event_id: String,
    body: String,
},
```

Route it from `TimelineCommand::SendReply` in the command handler.

- [x] **Step 5: Build the reply relation with public SDK APIs**

Inside the timeline actor send path, parse `in_reply_to_event_id` as an `OwnedEventId`, build:

```rust
let content =
    matrix_sdk::ruma::events::room::message::RoomMessageEventContentWithoutRelation::text_plain(
        body,
    );
```

Then build a `matrix_sdk::room::reply::Reply` and call:

```rust
let reply_content = timeline.room().make_reply_event(content, reply).await?;
let handle = timeline.send(reply_content.into()).await?;
```

Use `handle.transaction_id()` to populate `pending_sends` so `SendCompleted`
still correlates through the existing send-queue path. Classify errors through
the existing timeline failure path and never include body or event ids in error
text.

- [x] **Step 6: Add local QA reply stage**

In `scenario_reply`, after A sends root event `event1_id`, have B send a
`SendReply` command, wait for `SendCompleted` on B, wait for A to receive the
reply body, assert `in_reply_to_event_id` matches `event1_id`, and print:

```rust
println!("reply=ok");
```

`scenario_thread` remains explicitly unsupported until Task 5 lands.

- [x] **Step 7: Verify local lane**

```bash
cargo test -p matrix-desktop-core send_reply
npm --prefix apps/desktop run qa:headless-local -- --server=both --core --scenario=all
```

Expected: all local legs pass and print `reply=ok`.

- [ ] **Step 8: Commit**

```bash
git add crates/matrix-desktop-core/src/command.rs crates/matrix-desktop-core/src/timeline.rs crates/matrix-desktop-core/src/bin/headless-core-qa.rs
git commit -m "feat: add headless Matrix reply command"
```

## Task 5: Add Thread Reply Local Scenario

**Files:**
- Modify: `crates/matrix-desktop-core/src/bin/headless-core-qa.rs`
- Modify if needed: `crates/matrix-desktop-core/src/timeline.rs`

- [x] **Step 1: Write a local thread scenario using existing `TimelineKind::Thread`**

After the root event id is known, build:

```rust
let thread_key_b = TimelineKey {
    account_key: account_key_b.clone(),
    kind: matrix_desktop_core::ids::TimelineKind::Thread {
        room_id: room_id.clone(),
        root_event_id: event1_id.clone(),
    },
};
```

Subscribe B to `thread_key_b`, send `SendReply` through the thread key, and
assert A can subscribe to the same thread key and receive the reply item.
The reply item may require backward pagination when the initial snapshot does
not already contain it, and the QA must still verify that
`in_reply_to_event_id` matches the known root event id. The root event itself
is already proven by the room timeline flow and does not need to be reloaded
inside the thread-focused timeline.

- [x] **Step 2: Emit thread token**

When the thread timeline receives the reply and confirms its relation:

```rust
println!("thread=ok");
```

- [x] **Step 3: Verify**

```bash
npm --prefix apps/desktop run qa:headless-local -- --server=both --core --scenario=all
```

Expected: all local legs pass and print `thread=ok`.

- [ ] **Step 4: Commit**

```bash
git add crates/matrix-desktop-core/src/bin/headless-core-qa.rs crates/matrix-desktop-core/src/timeline.rs
git commit -m "qa: verify thread replies headlessly"
```

## Task 6: Add matrix.org Space Compatibility Stage

**Files:**
- Modify: `crates/matrix-desktop-core/src/bin/real-homeserver-qa.rs`
- Modify: `scripts/desktop-real-homeserver-qa.mjs`

- [ ] **Step 1: Add `--scenario` forwarding to real runner**

In `scripts/desktop-real-homeserver-qa.mjs`, parse:

```js
const scenarioOption = optionValue("--scenario") ?? "compat";
```

Pass:

```js
MATRIX_DESKTOP_REAL_QA_SCENARIO: scenarioOption
```

- [ ] **Step 2: Add real scenario parsing**

In `real-homeserver-qa.rs`:

```rust
const ENV_REAL_QA_SCENARIO: &str = "MATRIX_DESKTOP_REAL_QA_SCENARIO";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RealQaScenario {
    Compat,
    SpaceCompat,
    All,
}
```

Support `compat`, `space_compat`, and `all`.

- [ ] **Step 3: Implement single-account space create/set-child/cleanup**

Use the existing QA-created room. Create a synthetic QA space with a unique name, set the QA room as child, then treat the room-list projection as an optional observation instead of a gate. On matrix.org, the hard evidence is server acceptance of create and SetSpaceChild plus reliable leave/forget cleanup; the projection is only recorded when it shows up before the bounded probe times out.

Emit:

```rust
println!("real_space_create=ok");
println!("real_space_child=ok");
println!("real_space_projection=observed"); // or real_space_projection=not_observed
println!("real_space_cleanup=ok");
```

- [ ] **Step 4: Verify with matrix.org once**

Run only after local lanes are green:

```bash
npm --prefix apps/desktop run qa:real-homeserver -- --scenario=space_compat
```

Expected: one login, cleanup on success, no secret leakage, final summary still contains only `real_space_create=ok real_space_child=ok real_space_cleanup=ok`, and the transcript records `real_space_projection=observed` only when the bounded room-list probe sees it.

- [ ] **Step 5: Commit**

```bash
git add scripts/desktop-real-homeserver-qa.mjs crates/matrix-desktop-core/src/bin/real-homeserver-qa.rs
git commit -m "qa: add real homeserver space compatibility stage"
```

## Task 7: Add QA Aggregator Commands

**Files:**
- Modify: `apps/desktop/package.json`
- Modify: `docs/qa/headless-basic-operations.md`

- [x] **Step 1: Add package scripts**

In `apps/desktop/package.json`:

```json
"qa:headless-basic:local": "node ../../scripts/desktop-headless-local-qa.mjs --run --server=both --core --scenario=all",
"qa:headless-basic:real": "node ../../scripts/desktop-real-homeserver-qa.mjs --run --scenario=compat"
```

- [x] **Step 2: Update docs commands**

In `docs/qa/headless-basic-operations.md`, replace long commands with:

```bash
npm --prefix apps/desktop run qa:headless-basic:local
npm --prefix apps/desktop run qa:headless-basic:real
```

- [x] **Step 3: Verify**

```bash
npm --prefix apps/desktop run qa:headless-basic:local
npm --prefix apps/desktop run qa:secret-scan
node scripts/desktop-release-gate-check.mjs --no-compile
```

Expected: local QA passes; secret scan and release gate pass.

- [x] **Step 4: Commit**

```bash
git add apps/desktop/package.json docs/qa/headless-basic-operations.md
git commit -m "qa: add headless basic operations commands"
```

## Task 8: Add Linux Virtual-Display Client Basic Operations Smoke

**Files:**
- Modify: `scripts/desktop-linux-gui-qa.mjs`
- Modify: `apps/desktop/src/domain/qaTitle.ts`
- Modify: `apps/desktop/src/App.tsx`
- Modify: `apps/desktop/src/scripts/releaseScripts.test.ts`
- Modify: `docs/qa/headless-basic-operations.md`

- [x] **Step 1: Add a local-server GUI scenario option**

In `scripts/desktop-linux-gui-qa.mjs`, add:

```js
const guiScenario = optionValue("--scenario") ?? "signed-out";
```

Supported values:

```text
signed-out
local-login
local-send
```

`signed-out` keeps the existing Phase 15 smoke. `local-login` starts a
disposable local homeserver, registers a synthetic user, logs in through the
existing QA login transport, and waits for a ready synced client. `local-send`
does the same, then sends one message through the actual composer.

- [x] **Step 2: Reuse local homeserver startup safely**

Extract reusable server helpers from `scripts/desktop-headless-local-qa.mjs`
into a new script module:

```text
scripts/lib/local-homeserver-qa.mjs
```

Move these functions without behavior changes:

```js
checkInstalledHomeserver
conduitConfig
tuwunelConfig
startHomeserver
waitForHomeserver
registerUser
freePort
stopProcess
```

Import them from both `desktop-headless-local-qa.mjs` and
`desktop-linux-gui-qa.mjs`.

- [x] **Step 3: Add failing release script tests**

In `apps/desktop/src/scripts/releaseScripts.test.ts`, add:

```ts
test("linux GUI smoke supports local-login and local-send scenarios", () => {
  const source = readFileSync(
    new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
    "utf8"
  );

  expect(source).toContain("--scenario");
  expect(source).toContain("local-login");
  expect(source).toContain("local-send");
  expect(source).toContain("gui_local_login=ok");
  expect(source).toContain("gui_local_send=ok");
});
```

- [x] **Step 4: Run test to verify RED**

```bash
npm --prefix apps/desktop run test -- src/scripts/releaseScripts.test.ts
```

Expected: fails until the scenario strings and tokens are implemented.

- [x] **Step 5: Implement `local-login`**

For `--scenario=local-login`:

1. Start local Conduit by default; add `--server=conduit|tuwunel`.
2. Register one synthetic user.
3. Launch the Tauri app with:

```js
env.MATRIX_DESKTOP_QA_LOGIN_PIPE = join(dataDir, "qa-login.pipe");
env.MATRIX_DESKTOP_QA_FILE_CREDENTIAL_STORE_DIR = join(dataDir, "qa-credential-store");
env.MATRIX_DESKTOP_QA_TITLE = "1";
env.VITE_MATRIX_DESKTOP_QA_TITLE = "1";
```

4. Write the login payload to the FIFO using the same JSON shape the current
   GUI QA uses.
5. Wait for the QA title to satisfy:

```text
session=ready
sync=running
rooms>0
active_room=true
timeline_subscribed=true
errors=0
```

6. Print:

```text
gui_local_login=ok
```

- [x] **Step 6: Implement `local-send`**

For `--scenario=local-send`:

1. Run the `local-login` setup.
2. Use WebDriver to focus the composer:

```js
const composer = await browser.$('textarea[aria-label="Message composer"]');
await composer.waitForDisplayed({ timeout: timeoutMs });
await composer.setValue(`Matrix Desktop GUI QA ${timestamp()}`);
await browser.keys("Enter");
```

3. Wait for a QA title token:

```text
send=sent
errors=0
```

4. Print:

```text
gui_local_send=ok
```

- [x] **Step 7: Add product UI tokens only, no message bodies in logs**

Ensure `qaWindowTitle` exposes only counts/status tokens and never includes
message bodies, room IDs, event IDs, transaction IDs, or credentials. The
allowed additional tokens are:

```text
gui=local-login|local-send
send=idle|pending|sent|failed
```

- [x] **Step 8: Verify host loop**

Run from repo root:

```bash
node scripts/desktop-linux-gui-qa.mjs --check-tools
npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-login --server=conduit --artifact-dir=artifacts/linux-gui-local-login --timeout-ms=180000
npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-send --server=conduit --artifact-dir=artifacts/linux-gui-local-send --timeout-ms=180000
```

Expected output includes:

```text
gui_local_login=ok
gui_local_send=ok
notification_dbus=ok
window_state_path_contract=ok
```

Verified on 2026-06-13:
host local-login passed with `notification_dbus=ok`, `window_state_path_contract=ok`, `gui_local_login=ok`;
host local-send passed with `notification_dbus=ok`, `window_state_path_contract=ok`, `gui_local_send=ok`.

- [x] **Step 9: Verify Docker lane**

```bash
docker build -f docker/linux-gui.Dockerfile -t matrix-desktop-linux-gui:basic-ops .
docker run --rm --shm-size=2g -u "$(id -u):$(id -g)" \
  -v "$PWD:/work" \
  -v /tmp/matrix-desktop-cargo-home:/tmp/cargo-home \
  -v /tmp/matrix-desktop-gui-target:/tmp/matrix-desktop-gui-target \
  -v /tmp/matrix-desktop-npm-cache:/tmp/npm-cache \
  -w /work \
  -e HOME=/tmp \
  -e RUSTUP_HOME=/opt/rustup \
  -e CARGO_HOME=/tmp/cargo-home \
  -e CARGO_TARGET_DIR=/tmp/matrix-desktop-gui-target \
  -e NPM_CONFIG_CACHE=/tmp/npm-cache \
  -e PATH=/opt/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
  matrix-desktop-linux-gui:basic-ops \
  bash -c 'export RUSTC="$(rustup which rustc)"; export RUSTDOC="$(rustup which rustdoc)"; npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-send --server=conduit --artifact-dir=/work/artifacts/linux-gui-local-send-docker --timeout-ms=180000'
```

Expected: same tokens as host.

Verified on 2026-06-13:
`docker build -f docker/linux-gui.Dockerfile -t matrix-desktop-linux-gui:basic-ops .` passed;
Docker local-send passed with `notification_dbus=ok`, `window_state_path_contract=ok`, `gui_local_send=ok`;
the image now includes `conduit`, `tuwunel`, and `zstd`.

- [x] **Step 10: Commit**

```bash
git add scripts/desktop-linux-gui-qa.mjs scripts/lib/local-homeserver-qa.mjs scripts/desktop-headless-local-qa.mjs apps/desktop/src/domain/qaTitle.ts apps/desktop/src/App.tsx apps/desktop/src/scripts/releaseScripts.test.ts docs/qa/headless-basic-operations.md
git commit -m "qa: drive basic operations through Linux virtual display"
```

Verified on 2026-06-13 with commit `951056c` (`qa: drive Linux GUI basic operations locally`).

## Task 9: Expand Virtual-Display Client To Room/Space/Reply Controls

Gate review, 2026-06-13: deferred until the real product UI exposes the needed room/space/reply controls. Current audit shows the Add workspace button in `apps/desktop/src/App.tsx` has no handler, the room and space panels are read-only info panels, the thread composer is only a textarea and is not wired to send a reply, and `apps/desktop/src/backend/client.ts` exposes select/open/send text but no UI API for create room, create space, or reply. Do not treat Task 9 as implemented yet; keep the scenario steps unchecked. If the next pass proceeds without GUI controls, the remaining coverage path is headless Task 10.

**Files:**
- Modify after product controls exist: `scripts/desktop-linux-gui-qa.mjs`
- Modify after product controls exist: `apps/desktop/src/App.tsx`
- Modify after product controls exist: `apps/desktop/src/domain/qaTitle.ts`

- [ ] **Step 1: Add scenarios only after UI affordances exist**

Do not fake room/space/reply UI by direct CoreCommand injection. Add these
virtual-display scenarios only when the real Linux client exposes the relevant
controls:

```text
local-create-room
local-create-space
local-reply
local-thread
```

- [ ] **Step 2: Required tokens**

Each scenario must drive actual UI controls and print:

```text
gui_create_room=ok
gui_create_space=ok
gui_reply=ok
gui_thread=ok
```

- [ ] **Step 3: Server order**

Run Conduit first. Add Tuwunel after Conduit is stable:

```bash
npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-create-space --server=conduit
npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-create-space --server=tuwunel
```

- [ ] **Step 4: Commit each scenario separately**

Use one commit per UI operation family:

```bash
git commit -m "qa: verify room creation through Linux client"
git commit -m "qa: verify space creation through Linux client"
git commit -m "qa: verify replies through Linux client"
git commit -m "qa: verify threads through Linux client"
```

## Task 10: No-GUI Headless Acceptance

Task 10 was accepted without launching the GUI.

- Local headless basic operations were checked with `server=conduit` and
  `server=tuwunel`, and the final safe tokens were
  `safety=ok`, `login_sync=ok`, `room_space=ok`, `timeline=ok`,
  `reply=ok`, `thread=ok`, `edit_redact_search=ok`, `restore_cleanup=ok`.
- matrix.org headless compatibility was checked once through the package
  script's compat scenario with a one-login budget, and the final summary
  tokens were `login=ok`, `recovery=completed`, `sync=ok`, `rooms=5`,
  `spaces=1`, `dms=1`, `qa_room=created`, `send_msg1=ok`,
  `send_msg2=ok`, `edit_msg1=ok`, `redact_msg2=ok`,
  `paginate=end_reached`, `search=ok`, `store_restore=ok`,
  `restore_body=ok`, `leave_room=ok`, `forget_room=ok`,
  `logout=ok`, `post_logout_restore=not_found`.
- No GUI was launched for Task 10.

## Final Verification

Before claiming the plan complete, run:

```bash
cargo test -p matrix-desktop-core --lib
cargo test -p matrix-desktop-sdk -p matrix-desktop-state -p matrix-desktop-search -p matrix-desktop-key
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
cargo check --target wasm32-unknown-unknown -p matrix-desktop-state -p matrix-desktop-search
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run test
npm --prefix apps/desktop run test:ui-headless
npm --prefix apps/desktop run test:ipc-contract
npm --prefix apps/desktop run qa:secret-scan
node scripts/desktop-release-gate-check.mjs --no-compile
npm --prefix apps/desktop run qa:headless-basic:local
```

Then run the Linux virtual-display client lane:

```bash
npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-send --server=conduit --artifact-dir=artifacts/linux-gui-local-send --timeout-ms=180000
```

Run the real lane once when local is green and the login budget allows it:

```bash
npm --prefix apps/desktop run qa:headless-basic:real
```

If `real_space_cleanup=ok` has landed, also run:

```bash
npm --prefix apps/desktop run qa:real-homeserver -- --scenario=space_compat
```

## Stop Conditions

Stop and report instead of patching around the issue if any of these occurs:

- local server differs on semantics between Conduit and Tuwunel for the same operation,
- matrix.org creates a room/space that cannot be cleaned up reliably,
- reply/thread APIs require a canon change not covered by the command/event design,
- any transcript contains password, recovery key, access token, event body in Debug/error output, or an unexpected OS keychain prompt appears,
- matrix.org rate limits make the run exceed one login per QA attempt.
