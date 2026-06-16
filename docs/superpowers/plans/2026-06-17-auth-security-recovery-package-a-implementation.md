# Auth Security Recovery Package A Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement Package A from `2026-06-17-elementx-roadmap-design-batch.md`: OIDC/MAS auth, shared UIA/device sessions, E2EE key management, account management, and QR login with Rust-owned state machines before GUI work.

**Architecture:** Product state lives in Rust reducers and account actors. React renders snapshots and dispatches typed commands only. Secret-bearing values cross only command/native adapter boundaries and never enter reducer state, DTO snapshots, logs, QA output, or issue comments.

**Tech Stack:** Rust workspace (`matrix-desktop-state`, `matrix-desktop-core`, `matrix-desktop-sdk`), Tauri v2 commands/DTOs, React/TypeScript frontend, Vitest, Playwright browser-headless tests, local Conduit/Tuwunel headless QA, Linux virtual-display GUI QA.

---

## Scope

This plan covers Package A only:

- #37 OIDC / Matrix Authentication Service native auth.
- #38 Device / session manager, soft logout, shared UIA.
- #46 E2E key file export/import plus secure-backup setup.
- #47 Password account management, deactivation, 3PID, identity server.
- #45 QR login / sign in with another device.

Package B, Package C, and Package D require separate implementation plans after
Package A is reviewed or explicitly paused.

## Execution Rules

- Before starting each task, refresh open GitHub issues with:
  `gh issue list --state open --limit 100 --json number,title,updatedAt`.
- Keep commits scoped. Do not add issue-closing keywords until a child issue is
  fully complete and verified.
- Main agent owns shared files:
  `crates/matrix-desktop-state/src/state.rs`,
  `crates/matrix-desktop-state/src/action.rs`,
  `crates/matrix-desktop-state/src/reducer.rs`,
  `crates/matrix-desktop-state/src/effect.rs`,
  `crates/matrix-desktop-core/src/command.rs`,
  `crates/matrix-desktop-core/src/event.rs`,
  `crates/matrix-desktop-core/src/runtime.rs`,
  `crates/matrix-desktop-core/src/account.rs`,
  `apps/desktop/src-tauri/src/dto.rs`,
  `apps/desktop/src-tauri/src/commands.rs`,
  `apps/desktop/src/domain/types.ts`,
  `apps/desktop/src/backend/client.ts`,
  `apps/desktop/src/backend/browserFakeApi.ts`,
  `apps/desktop/src/test/appHarnessMain.tsx`,
  `apps/desktop/src/test/tauriIpcMock.ts`,
  `apps/desktop/src/App.tsx`,
  `apps/desktop/src/components/UserSettingsPanel.tsx`,
  `apps/desktop/src/i18n/messages.ts`,
  `apps/desktop/e2e/basic-operations.spec.ts`,
  `docs/architecture/state-machine.md`,
  `docs/architecture/overview.md`,
  `docs/policies/engineering-rules.md`,
  and `AGENTS.md`.
- Subagents may investigate SDK APIs or patch module-local helpers. Shared
  enum, DTO, reducer, command, and GUI files are integrated by the main agent.
- Use only private-data-free QA tokens. Do not print Matrix IDs, device IDs,
  IPs, URLs, access tokens, refresh tokens, passphrases, recovery keys,
  exported room-key file contents, file paths containing usernames, room IDs,
  event IDs, or message content.
- #46 room-key import/export uses the Matrix key-export file format used by
  Element clients. Use the public Matrix Rust SDK room-key export/import APIs
  and do not introduce a Ruri-specific JSON, archive, or wrapper file format.
  Synthetic fixture tests may assert the encrypted Megolm session data
  header/footer, but must never log or snapshot file contents from real
  accounts.

## Task 0: Package A Baseline And Issue Refresh

**Files:**
- Read: `REPOSITORY_RULES.md`
- Read: `AGENTS.md`
- Read: `docs/superpowers/specs/2026-06-17-elementx-roadmap-design-batch.md`
- Read: `docs/architecture/state-machine.md`
- Read: `docs/architecture/overview.md`

- [ ] **Step 1: Confirm local branch and dirty state**

Run:

```bash
git fetch origin
git status --short --branch
git log --oneline -5
```

Expected: branch is based on `origin/main`; the only unrelated dirty entry is
the pre-existing `vendor/matrix-rust-sdk` submodule if it is still present.

- [ ] **Step 2: Refresh Package A issues**

Run:

```bash
for n in 37 38 45 46 47; do
  gh issue view "$n" --json number,title,state,updatedAt,body,comments \
    > "/tmp/matrix-desktop-package-a-issue-$n.json"
done
gh issue view 12 --json number,title,state,updatedAt,body \
  > /tmp/matrix-desktop-package-a-issue-12.json
```

Expected: JSON files exist in `/tmp` only.

- [ ] **Step 3: Record start on #12**

Run:

```bash
gh issue comment 12 --body 'Package A implementation planning is approved and implementation is starting from docs/superpowers/plans/2026-06-17-auth-security-recovery-package-a-implementation.md. Scope: #37/#38/#45/#46/#47. Phase A Rust/state-machine work remains before GUI slices.'
```

Expected: #12 has a start marker.

## Task 1: Shared Package A State Skeleton

**Files:**
- Modify: `crates/matrix-desktop-state/src/state.rs`
- Modify: `crates/matrix-desktop-state/src/action.rs`
- Modify: `crates/matrix-desktop-state/src/reducer.rs`
- Modify: `crates/matrix-desktop-state/src/effect.rs`
- Modify: `crates/matrix-desktop-state/src/lib.rs`
- Test: `crates/matrix-desktop-state/tests/package_a_state.rs`
- Modify docs: `docs/architecture/state-machine.md`

- [ ] **Step 1: Write failing state skeleton tests**

Create `crates/matrix-desktop-state/tests/package_a_state.rs` with:

```rust
use matrix_desktop_state::{
    reduce, AccountManagementState, AppAction, AppState, AuthDiscoveryState,
    AuthFailureKind, DelegatedAuthLinks, DeviceSessionListState, E2eeKeyManagementState,
    LoginFlow, LoginFlowKind, QrLoginState, SecureBackupSetupState,
};

#[test]
fn auth_discovery_can_store_oidc_and_delegated_links_without_tokens() {
    let mut state = AppState::default();
    let effects = reduce(
        &mut state,
        AppAction::LoginDiscoverySucceeded {
            homeserver: "https://example.test".to_owned(),
            flows: vec![LoginFlow {
                kind: LoginFlowKind::Oidc,
                delegated_oidc_compatibility: true,
                display_name: Some("Provider".to_owned()),
            }],
            delegated: DelegatedAuthLinks {
                registration_url: Some("https://example.test/register".to_owned()),
                account_management_url: Some("https://example.test/account".to_owned()),
            },
        },
    );

    assert!(effects.iter().any(|effect| {
        matches!(
            effect,
            matrix_desktop_state::AppEffect::EmitUiEvent(
                matrix_desktop_state::UiEvent::AuthChanged
            )
        )
    }));
    assert_eq!(
        state.auth,
        AuthDiscoveryState::Ready {
            homeserver: "https://example.test".to_owned(),
            flows: vec![LoginFlow {
                kind: LoginFlowKind::Oidc,
                delegated_oidc_compatibility: true,
                display_name: Some("Provider".to_owned()),
            }],
            delegated: DelegatedAuthLinks {
                registration_url: Some("https://example.test/register".to_owned()),
                account_management_url: Some("https://example.test/account".to_owned()),
            },
        }
    );
}

#[test]
fn package_a_substates_start_secret_free_and_idle() {
    let state = AppState::default();
    assert!(matches!(
        state.e2ee_trust.key_management,
        E2eeKeyManagementState {
            room_key_export: matrix_desktop_state::RoomKeyExportState::Idle,
            room_key_import: matrix_desktop_state::RoomKeyImportState::Idle,
            secure_backup_setup: SecureBackupSetupState::Idle,
            passphrase_change: matrix_desktop_state::SecureBackupPassphraseChangeState::Idle,
        }
    ));
    assert_eq!(state.device_sessions, DeviceSessionListState::Idle);
    assert_eq!(state.account_management, AccountManagementState::Idle);
    assert_eq!(state.qr_login, QrLoginState::Idle);
}

#[test]
fn auth_failure_kind_is_coarse() {
    assert_eq!(format!("{:?}", AuthFailureKind::Network), "Network");
    assert_eq!(format!("{:?}", AuthFailureKind::Sdk), "Sdk");
}
```

- [ ] **Step 2: Run state test to verify it fails**

Run:

```bash
cargo test -p matrix-desktop-state --test package_a_state
```

Expected: FAIL because Package A types and new `LoginDiscoverySucceeded`
fields do not exist.

- [ ] **Step 3: Add state types**

In `crates/matrix-desktop-state/src/state.rs`, change `AuthDiscoveryState` and
`LoginFlow`, and add Package A states:

```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AuthDiscoveryState {
    Unknown,
    Discovering {
        homeserver: String,
    },
    Ready {
        homeserver: String,
        flows: Vec<LoginFlow>,
        #[serde(default)]
        delegated: DelegatedAuthLinks,
    },
    Failed {
        homeserver: String,
        #[serde(rename = "failureKind")]
        kind: AuthFailureKind,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct DelegatedAuthLinks {
    pub registration_url: Option<String>,
    pub account_management_url: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AuthFailureKind {
    Network,
    Unsupported,
    Cancelled,
    Forbidden,
    Timeout,
    Sdk,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LoginFlow {
    pub kind: LoginFlowKind,
    pub delegated_oidc_compatibility: bool,
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LoginFlowKind {
    Password,
    Sso,
    Oidc,
    Token,
    Unknown(String),
}
```

Add fields to `AppState`:

```rust
pub device_sessions: DeviceSessionListState,
pub account_management: AccountManagementState,
pub qr_login: QrLoginState,
```

Initialize them in `Default`:

```rust
device_sessions: DeviceSessionListState::Idle,
account_management: AccountManagementState::Idle,
qr_login: QrLoginState::Idle,
```

Add field to `E2eeTrustState`:

```rust
#[serde(default)]
pub key_management: E2eeKeyManagementState,
```

Add these enums and structs near the existing E2EE trust state:

```rust
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct E2eeKeyManagementState {
    pub room_key_export: RoomKeyExportState,
    pub room_key_import: RoomKeyImportState,
    pub secure_backup_setup: SecureBackupSetupState,
    pub passphrase_change: SecureBackupPassphraseChangeState,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RoomKeyExportState {
    #[default]
    Idle,
    Exporting { request_id: u64 },
    Exported { request_id: u64, exported_sessions: u64 },
    Failed { request_id: u64, #[serde(rename = "failureKind")] kind: TrustOperationFailureKind },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RoomKeyImportState {
    #[default]
    Idle,
    Importing { request_id: u64 },
    Imported { request_id: u64, imported_count: u64, total_count: u64 },
    Failed { request_id: u64, #[serde(rename = "failureKind")] kind: TrustOperationFailureKind },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SecureBackupSetupState {
    #[default]
    Idle,
    SettingUp { request_id: u64 },
    RecoveryKeyReady { request_id: u64, delivery: RecoveryKeyDeliveryState },
    Enabled { request_id: u64 },
    Failed { request_id: u64, #[serde(rename = "failureKind")] kind: TrustOperationFailureKind },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RecoveryKeyDeliveryState {
    #[default]
    NotWritten,
    Written,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SecureBackupPassphraseChangeState {
    #[default]
    Idle,
    Changing { request_id: u64 },
    Changed { request_id: u64, delivery: RecoveryKeyDeliveryState },
    Failed { request_id: u64, #[serde(rename = "failureKind")] kind: TrustOperationFailureKind },
}
```

Add device/account/QR states near account state definitions:

```rust
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DeviceSessionListState {
    #[default]
    Idle,
    Loading { request_id: u64 },
    Loaded { devices: Vec<DeviceSessionSummary> },
    Failed { request_id: u64, #[serde(rename = "failureKind")] kind: AuthFailureKind },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeviceSessionSummary {
    pub device_ordinal: u64,
    pub display_name: Option<String>,
    pub current: bool,
    pub verified: bool,
    pub inactive: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AccountManagementState {
    #[default]
    Idle,
    Working { request_id: u64, operation: AccountManagementOperation },
    AwaitingUia { request_id: u64, flow_id: u64, operation: AccountManagementOperation },
    Succeeded { request_id: u64, operation: AccountManagementOperation },
    Failed { request_id: u64, operation: AccountManagementOperation, #[serde(rename = "failureKind")] kind: AuthFailureKind },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AccountManagementOperation {
    RenameDevice,
    DeleteDevice,
    DeleteOtherDevices,
    ChangePassword,
    DeactivateAccount,
    ThreePid,
    IdentityServer,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum QrLoginState {
    #[default]
    Idle,
    CheckingCapability { request_id: u64 },
    Unavailable,
    Displaying { request_id: u64 },
    Scanning { request_id: u64 },
    Verified { request_id: u64 },
    Failed { request_id: u64, #[serde(rename = "failureKind")] kind: AuthFailureKind },
}
```

- [ ] **Step 4: Add actions/effects/reducer transitions**

In `action.rs`, add actions for request/success/failure of discovery, devices,
account management, key management, and QR state. Use only request IDs, counts,
operation enums, and failure kinds.

In `effect.rs`, add `UiEvent` variants:

```rust
DeviceSessionsChanged,
AccountManagementChanged,
QrLoginChanged,
E2eeKeyManagementChanged,
```

as `UiEvent` variants and emit them from reducer transitions.

In `reducer.rs`, add transitions so starting an operation changes Rust state
before actor routing. Reject a duplicate operation while the same sub-state is
running.

- [ ] **Step 5: Export new types**

In `crates/matrix-desktop-state/src/lib.rs`, export the new public types used
by Tauri and TypeScript contract tests.

- [ ] **Step 6: Run state tests**

Run:

```bash
cargo test -p matrix-desktop-state --test package_a_state
cargo test -p matrix-desktop-state --lib
```

Expected: both commands PASS.

- [ ] **Step 7: Update state-machine docs**

In `docs/architecture/state-machine.md`, add Package A sections:

```markdown
## Package A Auth And Security

OIDC/MAS, device sessions, shared UIA, key-management, account management, and
QR login are Rust-owned state machines. Their reducer state contains only
request ids, coarse failure kinds, capability facts, counts, and operation
status. Tokens, passphrases, recovery keys, device ids, IP addresses, exported
room-key file contents, account-management secrets, and rendezvous secrets are
command-boundary values only.
```

Add Mermaid diagrams for the four sub-state families:
auth discovery/login, UIA/account management, E2EE key management, and QR
login.

- [ ] **Step 8: Commit**

Run:

```bash
git add crates/matrix-desktop-state docs/architecture/state-machine.md
git commit -m "feat: add package A state skeleton"
```

Expected: commit succeeds.

## Task 2: Auth Discovery And OIDC/MAS Phase A

**Files:**
- Modify: `crates/matrix-desktop-sdk/src/lib.rs`
- Modify: `crates/matrix-desktop-core/src/command.rs`
- Modify: `crates/matrix-desktop-core/src/account.rs`
- Modify: `crates/matrix-desktop-core/src/runtime.rs`
- Modify: `apps/desktop/src-tauri/src/commands.rs`
- Modify: `apps/desktop/src-tauri/src/dto.rs`
- Modify: `apps/desktop/src/domain/types.ts`
- Modify: `apps/desktop/src/backend/client.ts`
- Modify: `apps/desktop/src/backend/client.test.ts`
- Test: `crates/matrix-desktop-sdk/tests/login_discovery.rs`
- Test: `crates/matrix-desktop-core/src/tests.rs`

- [ ] **Step 1: Add failing SDK discovery tests**

Extend `crates/matrix-desktop-sdk/tests/login_discovery.rs` with:

```rust
#[test]
fn oidc_login_flow_maps_to_desktop_oidc_without_tokens() {
    let flows = vec![matrix_desktop_sdk::MatrixLoginFlow {
        kind: matrix_desktop_sdk::MatrixLoginFlowKind::Oidc,
        delegated_oidc_compatibility: true,
        display_name: Some("Provider".to_owned()),
    }];
    let mapped = matrix_desktop_sdk::map_login_flows_to_desktop(flows);
    assert_eq!(mapped[0].kind, matrix_desktop_state::LoginFlowKind::Oidc);
    assert!(mapped[0].delegated_oidc_compatibility);
    assert_eq!(mapped[0].display_name.as_deref(), Some("Provider"));
}
```

Run:

```bash
cargo test -p matrix-desktop-sdk --test login_discovery oidc_login_flow_maps_to_desktop_oidc_without_tokens
```

Expected: FAIL because the SDK DTOs and mapper are not yet present.

- [ ] **Step 2: Add SDK DTOs and mapper**

In `crates/matrix-desktop-sdk/src/lib.rs`, add:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixLoginDiscovery {
    pub homeserver: String,
    pub flows: Vec<MatrixLoginFlow>,
    pub delegated: matrix_desktop_state::DelegatedAuthLinks,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixLoginFlow {
    pub kind: MatrixLoginFlowKind,
    pub delegated_oidc_compatibility: bool,
    pub display_name: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MatrixLoginFlowKind {
    Password,
    Sso,
    Oidc,
    Token,
    Unknown(String),
}

pub fn map_login_flows_to_desktop(flows: Vec<MatrixLoginFlow>) -> Vec<matrix_desktop_state::LoginFlow> {
    flows
        .into_iter()
        .map(|flow| matrix_desktop_state::LoginFlow {
            kind: match flow.kind {
                MatrixLoginFlowKind::Password => matrix_desktop_state::LoginFlowKind::Password,
                MatrixLoginFlowKind::Sso => matrix_desktop_state::LoginFlowKind::Sso,
                MatrixLoginFlowKind::Oidc => matrix_desktop_state::LoginFlowKind::Oidc,
                MatrixLoginFlowKind::Token => matrix_desktop_state::LoginFlowKind::Token,
                MatrixLoginFlowKind::Unknown(value) => matrix_desktop_state::LoginFlowKind::Unknown(value),
            },
            delegated_oidc_compatibility: flow.delegated_oidc_compatibility,
            display_name: flow.display_name,
        })
        .collect()
}
```

Add `discover_login(homeserver: &str)` using current SDK login-flow discovery
APIs. If the SDK lacks MAS account-management URL support in the public API,
return `DelegatedAuthLinks::default()` and document the gap in
`docs/upstream/matrix-rust-sdk-feedback.md`.

- [ ] **Step 3: Add core command**

In `command.rs`, add:

```rust
DiscoverLogin {
    request_id: RequestId,
    homeserver: String,
},
StartOidcLogin {
    request_id: RequestId,
    homeserver: String,
},
CompleteOidcLogin {
    request_id: RequestId,
    callback_url: String,
},
```

Add redacted `Debug` arms that print only the request id and a
`CallbackUrl(..)` placeholder.

- [ ] **Step 4: Route discovery through reducer and AccountActor**

In `runtime.rs`, project `AccountCommand::DiscoverLogin` to
`AppAction::LoginDiscoveryRequested`.

In `account.rs`, add handlers:

```rust
async fn handle_discover_login(&self, request_id: RequestId, homeserver: String)
async fn handle_start_oidc_login(&self, request_id: RequestId, homeserver: String)
async fn handle_complete_oidc_login(&mut self, request_id: RequestId, callback_url: String)
```

`handle_discover_login` calls the SDK wrapper and reduces either
`LoginDiscoverySucceeded { homeserver, flows, delegated }` or
`LoginDiscoveryFailed { homeserver, kind }`.

OIDC start/complete may initially return `AuthFailureKind::Unsupported` if the
vendored SDK path is unavailable. This still creates the Rust-owned contract
and avoids React-local OIDC semantics.

- [ ] **Step 5: Replace Tauri discovery shim**

In `apps/desktop/src-tauri/src/commands.rs`, replace the shim body of
`discover_login_methods` with a typed core command:

```rust
let mut event_conn = state.inner().runtime.attach();
let request_id = event_conn.next_request_id();
event_conn
    .command(build_discover_login_command(request_id, homeserver))
    .await
    .map_err(|e| format!("command submit failed: {e}"))?;
wait_for_auth_changed(&mut event_conn, request_id, LOGIN_EVENT_TIMEOUT).await?;
current_snapshot(state.inner()).await
```

Add:

```rust
pub(crate) fn build_discover_login_command(
    request_id: matrix_desktop_core::RequestId,
    homeserver: String,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::DiscoverLogin { request_id, homeserver })
}
```

- [ ] **Step 6: Update DTO and TS types**

Add `display_name` and `delegated` to `apps/desktop/src/domain/types.ts` and
verify `apps/desktop/src-tauri/src/dto.rs` serializes the new state shape.

- [ ] **Step 7: Add client tests**

In `apps/desktop/src/backend/client.test.ts`, add:

```ts
it("discovers login methods through typed Tauri command", async () => {
  const invoke = vi.fn().mockResolvedValue(snapshot);
  const api = createTauriClient(invoke);
  await api.discoverLoginMethods("https://example.test");
  expect(invoke).toHaveBeenCalledWith("discover_login_methods", {
    homeserver: "https://example.test",
  });
});
```

- [ ] **Step 8: Verify**

Run:

```bash
cargo test -p matrix-desktop-sdk --test login_discovery
cargo test -p matrix-desktop-core --lib
cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml --lib
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run test -- src/backend/client.test.ts
```

Expected: all commands PASS.

- [ ] **Step 9: Commit**

Run:

```bash
git add crates/matrix-desktop-sdk crates/matrix-desktop-core apps/desktop/src-tauri apps/desktop/src/domain apps/desktop/src/backend docs/upstream
git commit -m "feat: route login discovery through core"
```

Expected: commit succeeds.

## Task 3: Shared UIA And Device Session Manager Phase A

**Files:**
- Modify: `crates/matrix-desktop-sdk/src/lib.rs`
- Modify: `crates/matrix-desktop-core/src/command.rs`
- Modify: `crates/matrix-desktop-core/src/account.rs`
- Modify: `crates/matrix-desktop-core/src/runtime.rs`
- Modify: `crates/matrix-desktop-state/src/action.rs`
- Modify: `crates/matrix-desktop-state/src/reducer.rs`
- Test: `crates/matrix-desktop-state/tests/package_a_state.rs`
- Test: `crates/matrix-desktop-core/src/tests.rs`
- Modify docs: `docs/architecture/state-machine.md`

- [ ] **Step 1: Add failing reducer tests**

Extend `package_a_state.rs` with:

```rust
#[test]
fn device_session_query_sets_loading_and_emits_ui_event() {
    let mut state = AppState {
        session: matrix_desktop_state::SessionState::Ready(matrix_desktop_state::SessionInfo {
            homeserver: "https://example.test".to_owned(),
            user_id: "@user:example.test".to_owned(),
            device_id: "DEVICE".to_owned(),
        }),
        ..AppState::default()
    };
    let effects = reduce(&mut state, AppAction::DeviceSessionsQueryRequested { request_id: 77 });
    assert_eq!(state.device_sessions, DeviceSessionListState::Loading { request_id: 77 });
    assert!(effects.iter().any(|effect| {
        matches!(
            effect,
            matrix_desktop_state::AppEffect::EmitUiEvent(
                matrix_desktop_state::UiEvent::DeviceSessionsChanged
            )
        )
    }));
}
```

Run:

```bash
cargo test -p matrix-desktop-state --test package_a_state device_session_query_sets_loading_and_emits_ui_event
```

Expected: FAIL until actions and reducer transitions exist.

- [ ] **Step 2: Add device commands**

In `command.rs`, add:

```rust
QueryDevices { request_id: RequestId },
RenameDevice { request_id: RequestId, device_ordinal: u64, display_name: String },
DeleteDevices { request_id: RequestId, device_ordinals: Vec<u64>, auth: Option<matrix_desktop_state::IdentityResetAuthRequest> },
SoftLogoutReauth { request_id: RequestId, password: matrix_desktop_state::AuthSecret },
```

Use ordinals in UI/core state. Map ordinals to raw device IDs inside
`AccountActor` only.

- [ ] **Step 3: Add SDK wrappers**

In `matrix-desktop-sdk/src/lib.rs`, add:

```rust
pub struct MatrixDeviceSessionSummary {
    pub raw_device_id: String,
    pub display_name: Option<String>,
    pub current: bool,
    pub verified: bool,
    pub inactive: bool,
}

pub async fn list_devices(session: &MatrixClientSession) -> Result<Vec<MatrixDeviceSessionSummary>, E2eeTrustError>
pub async fn rename_device(session: &MatrixClientSession, raw_device_id: &str, display_name: &str) -> Result<(), E2eeTrustError>
pub async fn delete_devices(session: &MatrixClientSession, raw_device_ids: &[String], auth: Option<&IdentityResetAuthRequest>) -> Result<(), E2eeTrustError>
```

Map SDK device IDs to `DeviceSessionSummary` values in `AccountActor`. Populate
`device_ordinal`, `display_name`, `current`, `verified`, and `inactive`; keep
the raw ID map actor-private.

- [ ] **Step 4: Implement AccountActor private maps**

In `account.rs`, add an actor field:

```rust
device_session_ordinals: BTreeMap<u64, String>,
```

Clear it on logout, account switch, and actor shutdown.

- [ ] **Step 5: Verify**

Run:

```bash
cargo test -p matrix-desktop-state --test package_a_state
cargo test -p matrix-desktop-core --lib
cargo test -p matrix-desktop-sdk --lib
```

Expected: all commands PASS.

- [ ] **Step 6: Commit**

Run:

```bash
git add crates/matrix-desktop-state crates/matrix-desktop-core crates/matrix-desktop-sdk docs/architecture/state-machine.md
git commit -m "feat: add device session manager core"
```

Expected: commit succeeds.

## Task 4: #46 Room-Key Export/Import Phase A

**Files:**
- Modify: `crates/matrix-desktop-sdk/src/lib.rs`
- Modify: `crates/matrix-desktop-core/src/command.rs`
- Modify: `crates/matrix-desktop-core/src/account.rs`
- Modify: `crates/matrix-desktop-core/src/runtime.rs`
- Modify: `crates/matrix-desktop-state/src/action.rs`
- Modify: `crates/matrix-desktop-state/src/reducer.rs`
- Modify: `apps/desktop/src-tauri/src/commands.rs`
- Test: `crates/matrix-desktop-state/tests/e2ee_trust_state.rs`
- Test: `crates/matrix-desktop-core/src/tests.rs`

- [ ] **Step 1: Add failing key-management reducer tests**

Add to `crates/matrix-desktop-state/tests/e2ee_trust_state.rs`:

```rust
#[test]
fn room_key_export_request_sets_secret_free_pending_state() {
    let mut state = ready_state();
    let effects = reduce(&mut state, AppAction::RoomKeyExportRequested { request_id: 10 });
    assert_eq!(
        state.e2ee_trust.key_management.room_key_export,
        matrix_desktop_state::RoomKeyExportState::Exporting { request_id: 10 }
    );
    assert!(effects.iter().any(|effect| {
        matches!(
            effect,
            matrix_desktop_state::AppEffect::EmitUiEvent(
                matrix_desktop_state::UiEvent::E2eeKeyManagementChanged
            )
        )
    }));
}
```

Run:

```bash
cargo test -p matrix-desktop-state --test e2ee_trust_state room_key_export_request_sets_secret_free_pending_state
```

Expected: FAIL until actions/reducer transitions exist.

- [ ] **Step 2: Add command request structs**

In `command.rs`, add:

```rust
pub struct RoomKeyExportRequest {
    pub destination_path: PathBuf,
    pub passphrase: matrix_desktop_state::AuthSecret,
}

pub struct RoomKeyImportRequest {
    pub source_path: PathBuf,
    pub passphrase: matrix_desktop_state::AuthSecret,
}
```

Implement `Debug` so it prints only `DestinationPath(..)`,
`SourcePath(..)`, and `AuthSecret(..)`.

Add commands:

```rust
ExportRoomKeys { request_id: RequestId, request: RoomKeyExportRequest },
ImportRoomKeys { request_id: RequestId, request: RoomKeyImportRequest },
```

- [ ] **Step 3: Add SDK wrappers**

In `matrix-desktop-sdk/src/lib.rs`, add:

```rust
pub struct RoomKeyExportSummary {
    pub exported_sessions: u64,
}

pub struct RoomKeyImportSummary {
    pub imported_count: u64,
    pub total_count: u64,
}

pub async fn export_room_keys_to_file(
    session: &MatrixClientSession,
    path: PathBuf,
    passphrase: &AuthSecret,
) -> Result<RoomKeyExportSummary, E2eeTrustError>

pub async fn import_room_keys_from_file(
    session: &MatrixClientSession,
    path: PathBuf,
    passphrase: &AuthSecret,
) -> Result<RoomKeyImportSummary, E2eeTrustError>
```

Use `session.client().encryption().export_room_keys(path, passphrase.expose_secret(), |_| true).await`.
Use `import_room_keys(path, passphrase.expose_secret()).await` for import.
Do not read file bytes into React or DTO state. Do not implement custom file
serialization. The file must remain the Matrix key-export format used by
Element clients, as produced and consumed by the SDK, including the encrypted
Megolm session data header/footer.

- [ ] **Step 4: Project actor results**

In `account.rs`, add handlers:

```rust
async fn handle_export_room_keys(&self, request_id: RequestId, request: RoomKeyExportRequest)
async fn handle_import_room_keys(&self, request_id: RequestId, request: RoomKeyImportRequest)
```

Drop request structs immediately after SDK calls. Reduce only:

```rust
AppAction::RoomKeyExported { request_id, exported_sessions }
AppAction::RoomKeyImported { request_id, imported_count, total_count }
AppAction::RoomKeyExportFailed { request_id, kind }
AppAction::RoomKeyImportFailed { request_id, kind }
```

- [ ] **Step 5: Add Tauri commands**

In `commands.rs`, add:

```rust
#[tauri::command]
pub async fn export_room_keys(
    destination_path: String,
    passphrase: String,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String>

#[tauri::command]
pub async fn import_room_keys(
    source_path: String,
    passphrase: String,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String>
```

Register both commands in `apps/desktop/src-tauri/src/lib.rs`. The command
arguments are native/Tauri boundary values. They must not be logged.

- [ ] **Step 6: Verify**

Run:

```bash
cargo test -p matrix-desktop-state --test e2ee_trust_state
cargo test -p matrix-desktop-core --lib
cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml --lib
```

Expected: all commands PASS.

Also add a synthetic interoperability check that proves the export path produces
the Matrix/Element key-export envelope and the import path accepts that
envelope. The fixture must be synthetic and the test must not print the file
contents.

- [ ] **Step 7: Commit and update #46**

Run:

```bash
git add crates/matrix-desktop-state crates/matrix-desktop-core crates/matrix-desktop-sdk apps/desktop/src-tauri docs/architecture
git commit -m "feat: add room key file transfer core"
gh issue comment 46 --body 'Phase A1 progress: room-key export/import state, commands, SDK wrappers, and Tauri command boundaries landed. Secrets and file contents remain command-boundary only; reducer state carries request ids, counts, and coarse failure kinds. #46 remains open for secure-backup setup/passphrase change and GUI evidence.'
```

Expected: commit succeeds and #46 has a Phase A1 comment.

## Task 5: #46 Secure Backup Setup And Passphrase Change Phase A

**Files:**
- Modify: `crates/matrix-desktop-sdk/src/lib.rs`
- Modify: `crates/matrix-desktop-core/src/command.rs`
- Modify: `crates/matrix-desktop-core/src/account.rs`
- Modify: `crates/matrix-desktop-state/src/action.rs`
- Modify: `crates/matrix-desktop-state/src/reducer.rs`
- Modify: `apps/desktop/src-tauri/src/commands.rs`
- Test: `crates/matrix-desktop-state/tests/e2ee_trust_state.rs`
- Test: `crates/matrix-desktop-core/src/tests.rs`
- Modify docs: `AGENTS.md`
- Modify docs: `docs/policies/engineering-rules.md`

- [ ] **Step 1: Add failing reducer test**

Add:

```rust
#[test]
fn secure_backup_setup_recovery_key_ready_has_no_key_material() {
    let mut state = ready_state();
    reduce(&mut state, AppAction::SecureBackupSetupRequested { request_id: 33 });
    reduce(
        &mut state,
        AppAction::SecureBackupRecoveryKeyReady {
            request_id: 33,
            delivery: matrix_desktop_state::RecoveryKeyDeliveryState::Written,
        },
    );
    assert_eq!(
        state.e2ee_trust.key_management.secure_backup_setup,
        matrix_desktop_state::SecureBackupSetupState::RecoveryKeyReady {
            request_id: 33,
            delivery: matrix_desktop_state::RecoveryKeyDeliveryState::Written,
        }
    );
}
```

Run:

```bash
cargo test -p matrix-desktop-state --test e2ee_trust_state secure_backup_setup_recovery_key_ready_has_no_key_material
```

Expected: FAIL until secure-backup setup actions exist.

- [ ] **Step 2: Add command request structs**

In `command.rs`, add:

```rust
pub struct SecureBackupSetupRequest {
    pub passphrase: Option<matrix_desktop_state::AuthSecret>,
    pub recovery_key_destination_path: Option<PathBuf>,
}

pub struct SecureBackupPassphraseChangeRequest {
    pub old_secret: matrix_desktop_state::AuthSecret,
    pub new_passphrase: matrix_desktop_state::AuthSecret,
    pub recovery_key_destination_path: Option<PathBuf>,
}
```

Implement `Debug` with only booleans:

```rust
.field("has_passphrase", &self.passphrase.is_some())
.field("has_recovery_key_destination_path", &self.recovery_key_destination_path.is_some())
```

Add commands:

```rust
BootstrapSecureBackup { request_id: RequestId, request: SecureBackupSetupRequest },
ChangeSecureBackupPassphrase { request_id: RequestId, request: SecureBackupPassphraseChangeRequest },
```

- [ ] **Step 3: Add SDK functions**

In `matrix-desktop-sdk/src/lib.rs`, add:

```rust
pub struct SecureBackupSetupSummary {
    pub recovery_key_written: bool,
}

pub async fn bootstrap_secure_backup(
    session: &MatrixClientSession,
    passphrase: Option<&AuthSecret>,
    recovery_key_destination_path: Option<PathBuf>,
) -> Result<SecureBackupSetupSummary, E2eeTrustError>

pub async fn change_secure_backup_passphrase(
    session: &MatrixClientSession,
    old_secret: &AuthSecret,
    new_passphrase: &AuthSecret,
    recovery_key_destination_path: Option<PathBuf>,
) -> Result<SecureBackupSetupSummary, E2eeTrustError>
```

Implementation uses SDK recovery API:

```rust
let recovery = session.client().encryption().recovery();
let recovery_key = match passphrase {
    Some(passphrase) => recovery.enable().wait_for_backups_to_upload().with_passphrase(passphrase.expose_secret()).await?,
    None => recovery.enable().wait_for_backups_to_upload().await?,
};
write_recovery_key_if_requested(recovery_key, recovery_key_destination_path)?;
```

For passphrase change:

```rust
let recovery_key = recovery
    .recover_and_reset(old_secret.expose_secret())
    .with_passphrase(new_passphrase.expose_secret())
    .await?;
write_recovery_key_if_requested(recovery_key, recovery_key_destination_path)?;
```

`write_recovery_key_if_requested` writes the one-shot recovery key to the
native path and drops the string immediately after writing. It returns only
`recovery_key_written`.

- [ ] **Step 4: Document secret-delivery divergence from Element X**

In `AGENTS.md` and `docs/policies/engineering-rules.md`, add:

```markdown
- Secure-backup recovery keys may be produced by the SDK, but the product must
  not place the key in reducer state, Tauri DTO snapshots, React state, logs, QA
  tokens, screenshots, or issue comments. Desktop recovery-key delivery writes
  through a Rust/Tauri native artifact path and reports only a boolean or
  coarse status.
```

- [ ] **Step 5: Verify**

Run:

```bash
cargo test -p matrix-desktop-state --test e2ee_trust_state
cargo test -p matrix-desktop-core --lib
cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml --lib
npm --prefix apps/desktop run qa:secret-scan
```

Expected: all commands PASS.

- [ ] **Step 6: Commit and update #46**

Run:

```bash
git add crates/matrix-desktop-state crates/matrix-desktop-core crates/matrix-desktop-sdk apps/desktop/src-tauri AGENTS.md docs/policies docs/architecture
git commit -m "feat: add secure backup setup core"
gh issue comment 46 --body 'Phase A2 progress: secure-backup setup and passphrase-change state/commands landed. Recovery-key delivery is one-shot Rust/Tauri artifact handling; snapshots, DTOs, logs, React state, and QA tokens carry only request ids and delivery status. #46 remains open for GUI and Linux evidence.'
```

Expected: commit succeeds and #46 has a Phase A2 comment.

## Task 6: #37/#38/#47 GUI Command Surface

**Files:**
- Modify: `apps/desktop/src/backend/client.ts`
- Modify: `apps/desktop/src/backend/client.test.ts`
- Modify: `apps/desktop/src/backend/browserFakeApi.ts`
- Modify: `apps/desktop/src/backend/browserFakeApi.test.ts`
- Modify: `apps/desktop/src/test/tauriIpcMock.ts`
- Modify: `apps/desktop/src/test/appHarnessMain.tsx`
- Modify: `apps/desktop/src/domain/types.ts`
- Modify: `apps/desktop/src/i18n/messages.ts`
- Modify: `apps/desktop/src/components/UserSettingsPanel.tsx`
- Test: `apps/desktop/src/components/UserSettingsPanel.test.tsx`

- [ ] **Step 1: Add failing client tests**

In `apps/desktop/src/backend/client.test.ts`, add tests:

```ts
it("exports room keys through Tauri without logging the passphrase", async () => {
  const invoke = vi.fn().mockResolvedValue(snapshot);
  const api = createTauriClient(invoke);
  await api.exportRoomKeys("/tmp/export.txt", "passphrase");
  expect(invoke).toHaveBeenCalledWith("export_room_keys", {
    destinationPath: "/tmp/export.txt",
    passphrase: "passphrase",
  });
});

it("starts secure backup through Tauri", async () => {
  const invoke = vi.fn().mockResolvedValue(snapshot);
  const api = createTauriClient(invoke);
  await api.bootstrapSecureBackup("passphrase", "/tmp/recovery.txt");
  expect(invoke).toHaveBeenCalledWith("bootstrap_secure_backup", {
    passphrase: "passphrase",
    recoveryKeyDestinationPath: "/tmp/recovery.txt",
  });
});
```

Run:

```bash
npm --prefix apps/desktop run test -- src/backend/client.test.ts
```

Expected: FAIL until client methods exist.

- [ ] **Step 2: Add frontend API methods**

In `client.ts`, add:

```ts
exportRoomKeys(destinationPath: string, passphrase: string) {
  return invoke<DesktopSnapshot>("export_room_keys", { destinationPath, passphrase });
}
importRoomKeys(sourcePath: string, passphrase: string) {
  return invoke<DesktopSnapshot>("import_room_keys", { sourcePath, passphrase });
}
bootstrapSecureBackup(passphrase: string | null, recoveryKeyDestinationPath: string | null) {
  return invoke<DesktopSnapshot>("bootstrap_secure_backup", { passphrase, recoveryKeyDestinationPath });
}
changeSecureBackupPassphrase(oldSecret: string, newPassphrase: string, recoveryKeyDestinationPath: string | null) {
  return invoke<DesktopSnapshot>("change_secure_backup_passphrase", { oldSecret, newPassphrase, recoveryKeyDestinationPath });
}
```

- [ ] **Step 3: Update browser fake and harness**

In `browserFakeApi.ts` and `appHarnessMain.tsx`, implement fake command
responses that update only secret-free snapshot fields:

```ts
this.snapshot.state.e2ee_trust.key_management.room_key_export = {
  kind: "exported",
  request_id: requestId,
  exported_sessions: 1,
};
```

Do not store passphrase, file path, recovery key, or fixture contents.

- [ ] **Step 4: Add minimal UserSettingsPanel controls**

In `UserSettingsPanel.tsx`, add Security actions for export/import/setup.
Controls can use synthetic path text fields in browser-headless tests; native
file dialogs are a separate Tauri UX improvement. The state shown to users must
come from `snapshot.state.e2ee_trust.key_management`.

- [ ] **Step 5: Verify**

Run:

```bash
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run test -- src/backend/client.test.ts src/backend/browserFakeApi.test.ts src/components/UserSettingsPanel.test.tsx
```

Expected: all commands PASS.

- [ ] **Step 6: Commit**

Run:

```bash
git add apps/desktop/src/backend apps/desktop/src/test apps/desktop/src/domain apps/desktop/src/i18n apps/desktop/src/components
git commit -m "feat: expose package A security commands"
```

Expected: commit succeeds.

## Task 7: #46 Browser-Headless And Linux GUI Evidence

**Files:**
- Modify: `apps/desktop/e2e/basic-operations.spec.ts`
- Modify: `scripts/desktop-linux-gui-qa.mjs`
- Modify: `docs/qa/headless-basic-operations.md`
- Modify: `AGENTS.md`

- [ ] **Step 1: Add browser-headless GUI-operation test**

In `apps/desktop/e2e/basic-operations.spec.ts`, add:

```ts
test("security settings drive Rust-owned room-key transfer and secure backup state", async ({ page }) => {
  await bootHarness(page, "security-key-management");
  await page.getByRole("button", { name: /settings/i }).click();
  await page.getByRole("tab", { name: /security/i }).click();
  await page.getByLabel(/key export destination/i).fill("/tmp/ruri-export.txt");
  await page.getByLabel(/key export passphrase/i).fill("synthetic-passphrase");
  await page.getByRole("button", { name: /export room keys/i }).click();
  await expect(page.getByTestId("room-key-export-state")).toHaveText(/exported/i);
  await page.getByLabel(/recovery key destination/i).fill("/tmp/ruri-recovery.txt");
  await page.getByLabel(/secure backup passphrase/i).fill("synthetic-passphrase");
  await page.getByRole("button", { name: /set up secure backup/i }).click();
  await expect(page.getByTestId("secure-backup-state")).toHaveText(/recovery key saved/i);
});
```

Run:

```bash
npm --prefix apps/desktop run test:ui-headless -- e2e/basic-operations.spec.ts --grep "security settings drive Rust-owned room-key transfer"
```

Expected: FAIL until UI selectors and fake runtime are wired.

- [ ] **Step 2: Add Linux GUI scenario tokens**

In `scripts/desktop-linux-gui-qa.mjs`, add a `local-e2ee-key-management`
scenario that drives Security settings and asserts these tokens:

```text
gui_room_key_export=ok
gui_room_key_import=ok
gui_secure_backup_setup=ok
```

The scenario must not print paths, passphrases, recovery keys, Matrix IDs,
device IDs, room IDs, event IDs, or message contents.

- [ ] **Step 3: Verify**

Run:

```bash
npm --prefix apps/desktop run test:ui-headless -- e2e/basic-operations.spec.ts --grep "security settings drive Rust-owned room-key transfer"
node --check scripts/desktop-linux-gui-qa.mjs
```

Run Linux virtual-display when host tools are available:

```bash
PATH=/tmp/matrix-desktop-local-qa-bin:$PATH \
npm --prefix apps/desktop run qa:linux-gui -- \
  --scenario=local-e2ee-key-management \
  --server=conduit \
  --artifact-dir=artifacts/linux-gui-local-e2ee-key-management-host \
  --timeout-ms=180000
```

Expected: browser-headless passes; Linux scenario emits all three tokens.

- [ ] **Step 4: Commit and close #46 if complete**

Run:

```bash
git add apps/desktop/e2e scripts docs/qa AGENTS.md
git commit -m "test: add e2ee key management GUI evidence"
```

If #46 acceptance is satisfied, run:

```bash
gh issue comment 46 --body 'Completed #46. Evidence: Rust state tests, core tests, SDK checks, Tauri lib check, frontend typecheck, browser-headless security key-management test, Linux virtual-display local-e2ee-key-management tokens, and secret scan. Key management remains Rust-owned and no key material or secrets appear in QA output.'
gh issue close 46
```

Expected: commit succeeds; #46 closes only if evidence is present.

## Task 8: #47 Account Management Phase A/B

**Files:**
- Modify: `crates/matrix-desktop-sdk/src/lib.rs`
- Modify: `crates/matrix-desktop-core/src/command.rs`
- Modify: `crates/matrix-desktop-core/src/account.rs`
- Modify: `apps/desktop/src-tauri/src/commands.rs`
- Modify: `apps/desktop/src/components/UserSettingsPanel.tsx`
- Test: `crates/matrix-desktop-core/src/tests.rs`
- Test: `apps/desktop/src/components/UserSettingsPanel.test.tsx`

- [ ] **Step 1: Add account-management commands**

Add `AccountCommand` variants:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThreePidMedium {
    Email,
    Phone,
}

ChangePassword { request_id: RequestId, old_password: AuthSecret, new_password: AuthSecret },
DeactivateAccount { request_id: RequestId, erase: bool, auth: Option<IdentityResetAuthRequest> },
AddThreePid { request_id: RequestId, medium: ThreePidMedium, address: String },
RemoveThreePid { request_id: RequestId, medium: ThreePidMedium, address_ordinal: u64 },
SetIdentityServer { request_id: RequestId, server: Option<String> },
```

Add `Debug` implementations that redact passwords and addresses.

- [ ] **Step 2: Gate OIDC accounts to external account management**

Use `AuthDiscoveryState::Ready.delegated.account_management_url` and session
account mode facts to render an external Manage account link for OIDC accounts.
Password homeservers render native password/3PID/deactivation controls.

- [ ] **Step 3: Verify**

Run:

```bash
cargo test -p matrix-desktop-core --lib
cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml --lib
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run test -- src/components/UserSettingsPanel.test.tsx
```

Expected: all commands PASS.

- [ ] **Step 4: Commit and update #47**

Run:

```bash
git add crates/matrix-desktop-core crates/matrix-desktop-sdk apps/desktop/src-tauri apps/desktop/src/components apps/desktop/src/i18n
git commit -m "feat: add account management state and commands"
gh issue comment 47 --body 'Account management Phase A/B progress landed: password-account commands are Rust-owned and OIDC accounts delegate to the account-management URL. Secrets are command-boundary only. Issue remains open if 3PID or identity-server Linux evidence is not yet present.'
```

Expected: commit succeeds.

## Task 9: #45 QR Login Contract

**Files:**
- Modify: `crates/matrix-desktop-state/src/state.rs`
- Modify: `crates/matrix-desktop-core/src/command.rs`
- Modify: `crates/matrix-desktop-core/src/account.rs`
- Modify: `apps/desktop/src-tauri/src/commands.rs`
- Modify: `apps/desktop/src/components/UserSettingsPanel.tsx`
- Test: `crates/matrix-desktop-state/tests/package_a_state.rs`
- Test: `crates/matrix-desktop-core/src/tests.rs`
- Test: `apps/desktop/e2e/basic-operations.spec.ts`

- [ ] **Step 1: Implement mockable QR state machine**

Add commands:

```rust
CheckQrLoginCapability { request_id: RequestId },
StartQrLoginDisplay { request_id: RequestId },
StartQrLoginScan { request_id: RequestId },
CancelQrLogin { request_id: RequestId },
```

Reducer transitions:

```text
Idle -> CheckingCapability -> Unavailable
Idle -> Displaying -> Verified
Idle -> Scanning -> Verified
Any running state -> Idle on cancel/logout/account switch
Any running state -> Failed on coarse failure
```

Use a mock channel in browser-headless tests. Real MSC4108 rendezvous transport
is behind SDK/native capability and can return `Unavailable` until a public SDK
path is wired.

- [ ] **Step 2: Add GUI proof**

Add Settings/Security controls:

```text
Show QR to sign in another device
Sign in with QR
Cancel QR login
```

The displayed QR payload is a redacted placeholder in fake/headless mode. Real
rendezvous secrets are native/actor-private and never logged.

- [ ] **Step 3: Verify**

Run:

```bash
cargo test -p matrix-desktop-state --test package_a_state
cargo test -p matrix-desktop-core --lib
npm --prefix apps/desktop run test:ui-headless -- e2e/basic-operations.spec.ts --grep "QR login"
```

Expected: all commands PASS.

- [ ] **Step 4: Commit and update #45**

Run:

```bash
git add crates/matrix-desktop-state crates/matrix-desktop-core apps/desktop/src-tauri apps/desktop/src/components apps/desktop/e2e docs/architecture
git commit -m "feat: add QR login state contract"
gh issue comment 45 --body 'QR login state contract and GUI proof landed with a mockable Rust-owned rendezvous state machine. Real MSC4108 transport remains capability-gated and private-data-free. Issue remains open until real transport or approved unavailable-state acceptance is recorded.'
```

Expected: commit succeeds.

## Task 10: Package A Final Gates And Issue Accounting

**Files:**
- Modify: `docs/architecture/overview.md`
- Modify: `docs/architecture/state-machine.md`
- Modify: `docs/policies/engineering-rules.md`
- Modify: `AGENTS.md`
- Modify issue comments: #37, #38, #45, #46, #47, #12

- [ ] **Step 1: Run full Package A verification**

Run:

```bash
cargo test -p matrix-desktop-state --lib
cargo test -p matrix-desktop-state --test package_a_state
cargo test -p matrix-desktop-state --test e2ee_trust_state
cargo test -p matrix-desktop-core --lib
cargo test -p matrix-desktop-sdk --lib
cargo test -p matrix-desktop-sdk --test login_discovery
cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml --lib
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run test -- src/backend/client.test.ts src/backend/browserFakeApi.test.ts src/components/UserSettingsPanel.test.tsx
npm --prefix apps/desktop run qa:secret-scan
cargo fmt --check
git diff --check
```

Expected: all commands PASS.

- [ ] **Step 2: Run focused browser-headless**

Run:

```bash
npm --prefix apps/desktop run test:ui-headless -- e2e/basic-operations.spec.ts --grep "security|QR|session|account"
```

Expected: Package A browser-headless checks PASS.

- [ ] **Step 3: Run Linux GUI lanes where implemented**

Run the Linux scenarios added by this plan:

```bash
PATH=/tmp/matrix-desktop-local-qa-bin:$PATH \
npm --prefix apps/desktop run qa:linux-gui -- \
  --scenario=local-e2ee-key-management \
  --server=conduit \
  --artifact-dir=artifacts/linux-gui-local-e2ee-key-management-host \
  --timeout-ms=180000
```

Expected: `gui_room_key_export=ok`, `gui_room_key_import=ok`, and
`gui_secure_backup_setup=ok` appear.

- [ ] **Step 4: Update issues**

Close only issues whose acceptance is fully proven. Use this pattern:

```bash
gh issue comment ISSUE_NUMBER --body 'Completed. Evidence: COMMAND_LIST_AND_TOKENS. Product state remains Rust-owned; React renders only; QA output is private-data-free.'
gh issue close ISSUE_NUMBER
```

For incomplete issues, comment with remaining checkboxes and evidence gathered.

- [ ] **Step 5: Update umbrella #12**

Edit #12 to add Package A progress under the reconciled inventory. Do not mark
the umbrella complete unless every promoted child issue and #9/#31 are complete.

- [ ] **Step 6: Commit docs and issue-accounting updates**

Run:

```bash
git add docs AGENTS.md
git commit -m "docs: record package A implementation evidence"
```

Expected: commit succeeds if docs changed. If no docs changed, skip this commit
and record why in the final Package A summary.
