#!/usr/bin/env node

import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, test } from "node:test";

import {
  coreLocalIntegrationTargets,
  findLeafCrateBoundaryViolations,
  testkitTargets
} from "./check-leaf-crate-boundaries.mjs";

const roots = [];
afterEach(() => {
  for (const root of roots.splice(0)) fs.rmSync(root, { recursive: true, force: true });
});

function write(root, relativePath, source) {
  const absolutePath = path.join(root, relativePath);
  fs.mkdirSync(path.dirname(absolutePath), { recursive: true });
  fs.writeFileSync(absolutePath, source);
}

function fixture() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "koushi-leaf-boundary-"));
  roots.push(root);
  write(
    root,
    "Cargo.toml",
    '[workspace]\nmembers = ["crates/koushi-store", "crates/koushi-core", "crates/koushi-core-testkit", "crates/koushi-search", "crates/koushi-media", "crates/koushi-qa"]\ndefault-members = ["crates/koushi-store", "crates/koushi-core"]\n'
  );
  write(
    root,
    "crates/koushi-store/Cargo.toml",
    '[package]\nname = "koushi-store"\n[dependencies]\nkoushi-key = { path = "../koushi-key" }\n[features]\ntest-hooks = []\n'
  );
  write(
    root,
    "crates/koushi-store/src/lib.rs",
    "pub enum CredentialStoreBackend {}\npub struct CredentialVaultFile;\npub fn encrypt_envelope() {}\npub fn decrypt_envelope() {}\npub fn atomic_replace_file() {}\n"
  );
  write(
    root,
    "crates/koushi-search/Cargo.toml",
    '[package]\nname = "koushi-search"\n[dependencies]\nkoushi-state = { path = "../koushi-state" }\n'
  );
  write(root, "crates/koushi-search/src/lib.rs", "pub struct SearchDocumentStore;\n");
  write(
    root,
    "crates/koushi-media/Cargo.toml",
    '[package]\nname = "koushi-media"\n[dependencies]\nimage = "0.25"\n'
  );
  write(
    root,
    "crates/koushi-media/src/lib.rs",
    "pub struct ImageKind;\npub fn image_kind() {}\n"
  );
  write(
    root,
    "crates/koushi-core/Cargo.toml",
    '[package]\nname = "koushi-core"\n[dependencies]\nkoushi-store = { path = "../koushi-store" }\n[features]\ntest-hooks = ["koushi-store/test-hooks"]\n'
  );
  write(
    root,
    "crates/koushi-core/src/lib.rs",
    "pub struct StoreActor;\npub(crate) const ACTOR_MESSAGE_QUEUE_CAPACITY: usize = 1;\n"
  );
  write(
    root,
    "crates/koushi-core/src/renderable_thumbnail.rs",
    "pub fn thumbnail() { koushi_media::image_kind(); }\n"
  );
  write(
    root,
    "crates/koushi-core/src/composer_draft_lifecycle.rs",
    "pub(crate) struct ForwardedComposerDraftPermit;\n"
  );
  write(root, "crates/koushi-core/src/runtime.rs", "pub struct CoreRuntime;\n");
  write(root, "crates/koushi-core/src/runtime/composer.rs", "pub fn composer() {}\n");
  write(
    root,
    "crates/koushi-core/src/command_policy.rs",
    "pub(crate) fn space_member_forward_failure_action() {}\n"
  );
  write(root, "crates/koushi-core/src/account/actor.rs", "pub struct AccountActorHandle;\n");
  write(
    root,
    "crates/koushi-core/src/account/direct_action_ledger.rs",
    "pub(super) struct DirectActionLedger;\n"
  );
  write(root, "crates/koushi-mcp-host/src/handlers.rs", "pub fn edit_message() {}\n");
  write(
    root,
    "crates/koushi-core/src/timeline/recovery_model.rs",
    "pub struct RecoveryOperation;\n"
  );
  for (const featurePath of [
    "crates/koushi-core/src/scheduled_send.rs",
    "crates/koushi-core/src/account/scheduled_send.rs",
    "crates/koushi-core/src/runtime/scheduled_send.rs",
    "crates/koushi-core/src/store/scheduled_sends.rs",
    "crates/koushi-core/src/timeline/composer.rs",
    "crates/koushi-core/src/store/composer_drafts.rs",
    "crates/koushi-core/src/runtime/navigation.rs",
    "crates/koushi-core/src/timeline/navigation.rs",
    "crates/koushi-core/src/store/navigation.rs"
  ]) {
    write(root, featurePath, "pub fn owner() {}\n");
  }
  write(
    root,
    "crates/koushi-core-testkit/Cargo.toml",
    '[package]\nname = "koushi-core-testkit"\npublish = false\n[dev-dependencies]\nkoushi-core = { path = "../koushi-core", features = ["test-hooks"] }\n[target.\'cfg(unix)\'.dev-dependencies]\nhelper = { path = "../helper" }\n'
  );
  write(root, "crates/koushi-core-testkit/tests/support/mod.rs", "pub fn fixture() {}\n");
  for (const target of testkitTargets) {
    write(root, path.join("crates/koushi-core-testkit/tests", target), "#[test]\nfn moved() {}\n");
  }
  for (const target of coreLocalIntegrationTargets) {
    write(root, path.join("crates/koushi-core/tests", target), "#[test]\nfn local() {}\n");
  }
  write(
    root,
    ".github/workflows/ci.yml",
    "- run: cargo test -p koushi-core-testkit\n"
  );
  write(
    root,
    "crates/koushi-qa/Cargo.toml",
    '[package]\nname = "koushi-qa"\n[dependencies]\nkoushi-store = { path = "../koushi-store" }\n[features]\nqa-bin = ["koushi-store/test-hooks"]\n'
  );
  write(
    root,
    "crates/koushi-qa/src/lib.rs",
    "pub fn uses_store_probe() { koushi_store::resolved_credential_backend_is_file_dir(); }\n"
  );
  return root;
}

test("accepts the intended persistence and Core testkit boundaries", () => {
  assert.deepEqual(findLeafCrateBoundaryViolations(fixture()), []);
});

test("accepts CI running koushi-core-testkit through an unexcluded workspace test", () => {
  const root = fixture();
  fs.writeFileSync(
    path.join(root, ".github/workflows/ci.yml"),
    "- run: cargo test --profile ci --workspace --exclude sidebar-composition --exclude key-management\n"
  );
  assert.deepEqual(findLeafCrateBoundaryViolations(root), []);
});

test("rejects a workspace test that excludes koushi-core-testkit without an explicit run", () => {
  const root = fixture();
  fs.writeFileSync(
    path.join(root, ".github/workflows/ci.yml"),
    "- run: cargo test --profile ci --workspace --exclude koushi-core-testkit\n"
  );
  assert(findLeafCrateBoundaryViolations(root).includes("CI must run koushi-core-testkit"));
});

test("detects missing packages, dependency leaks, and retained Core crypto", () => {
  const root = fixture();
  fs.writeFileSync(
    path.join(root, "Cargo.toml"),
    '[workspace]\nmembers = ["crates/koushi-core"]\ndefault-members = ["crates/koushi-core"]\n'
  );
  fs.appendFileSync(path.join(root, "crates/koushi-store/Cargo.toml"), 'tokio = "1"\n');
  fs.appendFileSync(
    path.join(root, "crates/koushi-core/Cargo.toml"),
    'chacha20poly1305 = "0.10"\n'
  );
  fs.appendFileSync(
    path.join(root, "crates/koushi-core/src/lib.rs"),
    'use chacha20poly1305::ChaCha20Poly1305;\npub enum CredentialStoreBackend {}\n#[cfg(feature = "test-hooks")]\npub fn unit_test_hidden() {}\n'
  );

  const violations = findLeafCrateBoundaryViolations(root);
  assert(violations.includes("workspace member missing: crates/koushi-store"));
  assert(violations.includes("default workspace member missing: crates/koushi-store"));
  assert(violations.includes("koushi-store has forbidden dependency: tokio"));
  assert(violations.includes("koushi-core still depends on chacha20poly1305"));
  assert(violations.some((item) => item.includes("CredentialStoreBackend")));
  assert(violations.some((item) => item.includes("use chacha20poly1305")));
  assert(violations.some((item) => item.includes("Core test-hook cfg excludes unit tests")));
});

test("detects pure search/media dependency and classifier ownership violations", () => {
  const root = fixture();
  fs.appendFileSync(
    path.join(root, "crates/koushi-search/Cargo.toml"),
    'tokio = "1"\n'
  );
  fs.appendFileSync(
    path.join(root, "crates/koushi-media/Cargo.toml"),
    'koushi-core = { path = "../koushi-core" }\n'
  );
  fs.writeFileSync(
    path.join(root, "crates/koushi-search/src/lib.rs"),
    "pub fn bad(_: std::path::PathBuf) { tokio::spawn(async {}); }\n"
  );
  fs.writeFileSync(path.join(root, "crates/koushi-media/src/lib.rs"), "pub fn missing() {}\n");
  write(root, "crates/koushi-core/src/cached_image.rs", "pub fn cached_image_kind() {}\n");
  fs.writeFileSync(
    path.join(root, "crates/koushi-core/src/renderable_thumbnail.rs"),
    "pub fn stale() {}\n"
  );

  const violations = findLeafCrateBoundaryViolations(root);
  assert(violations.includes("koushi-search has forbidden dependency: tokio"));
  assert(violations.includes("koushi-media has forbidden dependency: koushi-core"));
  assert(violations.some((item) => item.includes("koushi-search source contains forbidden token")));
  assert(violations.includes("koushi-media definition missing: pub struct ImageKind"));
  assert(violations.includes("koushi-media definition missing: pub fn image_kind"));
  assert(violations.includes("cached image classifier remains in koushi-core"));
  assert(violations.some((item) => item.includes("image kind classifier function remains in Core")));
  assert(violations.includes("Core renderable thumbnails must use koushi-media image_kind"));
});

test("detects Core runtime reverse edges, concrete child construction, and misplaced owners", () => {
  const root = fixture();
  fs.writeFileSync(
    path.join(root, "crates/koushi-core/src/account/actor.rs"),
    "use crate::runtime::CoreRuntime;\ntrait ActorFactory {}\nfn spawn() { RoomActor::spawn(); }\n"
  );
  fs.writeFileSync(
    path.join(root, "crates/koushi-core/src/runtime.rs"),
    "const ACTOR_MESSAGE_QUEUE_CAPACITY: usize = 1;\nfn space_member_forward_failure_action() {}\n"
  );
  fs.writeFileSync(
    path.join(root, "crates/koushi-core/src/runtime/composer.rs"),
    "struct ForwardedComposerDraftPermit;\n"
  );
  write(root, "crates/koushi-core/src/room_key_recovery.rs", "pub struct RecoveryOperation;\n");
  fs.rmSync(path.join(root, "crates/koushi-core/src/timeline/recovery_model.rs"));
  fs.rmSync(path.join(root, "crates/koushi-core/src/runtime/navigation.rs"));

  const violations = findLeafCrateBoundaryViolations(root);
  assert(violations.some((item) => item.includes("imports runtime internals")));
  assert(violations.some((item) => item.includes("constructs a concrete child actor")));
  assert(violations.some((item) => item.includes("generic actor framework")));
  assert(violations.includes("struct ForwardedComposerDraftPermit remains in runtime composer"));
  assert(violations.includes("const ACTOR_MESSAGE_QUEUE_CAPACITY remains in runtime"));
  assert(violations.includes("fn space_member_forward_failure_action remains in runtime"));
  assert(violations.includes("room-key recovery model remains at the Core crate root"));
  assert(violations.includes("timeline recovery model owner is missing RecoveryOperation"));
  assert(
    violations.includes(
      "owner-local feature layer missing: crates/koushi-core/src/runtime/navigation.rs"
    )
  );
});

test("detects a message-action retry ledger outside Core", () => {
  const root = fixture();
  fs.rmSync(path.join(root, "crates/koushi-core/src/account/direct_action_ledger.rs"));
  write(
    root,
    "crates/koushi-mcp-host/src/actions.rs",
    "pub(crate) const LEDGER_TTL: u64 = 600;\npub(crate) struct ActionLedger;\n"
  );

  const violations = findLeafCrateBoundaryViolations(root);
  assert(
    violations.includes("Core direct action ledger definition missing: struct DirectActionLedger")
  );
  assert(
    violations.includes(
      "MCP host defines a message-action retry ledger: crates/koushi-mcp-host/src/actions.rs"
    )
  );
});

test("detects missing test-hook propagation and stale QA probe routing", () => {
  const root = fixture();
  fs.writeFileSync(
    path.join(root, "crates/koushi-core/Cargo.toml"),
    '[package]\nname = "koushi-core"\n[dependencies]\nkoushi-store = { path = "../koushi-store" }\n[features]\ntest-hooks = []\n'
  );
  fs.writeFileSync(
    path.join(root, "crates/koushi-qa/Cargo.toml"),
    '[package]\nname = "koushi-qa"\n[features]\nqa-bin = []\n'
  );
  fs.writeFileSync(
    path.join(root, "crates/koushi-qa/src/lib.rs"),
    "pub fn stale() { koushi_core::store::resolved_credential_backend_is_file_dir(); }\n"
  );

  const violations = findLeafCrateBoundaryViolations(root);
  assert(violations.includes("koushi-core test-hooks must enable koushi-store/test-hooks"));
  assert(violations.includes("koushi-qa must depend directly on koushi-store"));
  assert(violations.includes("koushi-qa qa-bin must enable koushi-store/test-hooks"));
  assert(violations.some((item) => item.includes("removed Core credential probe")));
});

test("detects direct and target-specific testkit build dependencies", () => {
  for (const header of [
    "[build-dependencies]",
    "[build-dependencies.helper]",
    "[target.'cfg(unix)'.build-dependencies]"
  ]) {
    const root = fixture();
    fs.writeFileSync(
      path.join(root, "crates/koushi-core-testkit/Cargo.toml"),
      `[package]\nname = "koushi-core-testkit"\npublish = false\n${header}\npath = "../helper"\n[dev-dependencies]\nkoushi-core = { path = "../koushi-core", features = ["test-hooks"] }\n`
    );
    assert(
      findLeafCrateBoundaryViolations(root).includes(
        "koushi-core-testkit must use dev-dependencies only"
      )
    );
  }
});

test("detects target-specific normal testkit dependencies", () => {
  const root = fixture();
  fs.writeFileSync(
    path.join(root, "crates/koushi-core-testkit/Cargo.toml"),
    '[package]\nname = "koushi-core-testkit"\npublish = false\n[target.\'cfg(unix)\'.dependencies]\nkoushi-qa = { path = "../koushi-qa" }\n[dev-dependencies]\nkoushi-core = { path = "../koushi-core", features = ["test-hooks"] }\n'
  );
  assert(
    findLeafCrateBoundaryViolations(root).includes(
      "koushi-core-testkit must use dev-dependencies only"
    )
  );
});

test("detects testkit default or production leakage, self-dependency, and missing targets", () => {
  const root = fixture();
  fs.appendFileSync(
    path.join(root, "crates/koushi-core/Cargo.toml"),
    'koushi-core = { path = ".", features = ["test-hooks"] }\nkoushi-core-testkit = { path = "../koushi-core-testkit" }\n'
  );
  fs.appendFileSync(
    path.join(root, "crates/koushi-qa/Cargo.toml"),
    'koushi-core-testkit = { path = "../koushi-core-testkit" }\n'
  );
  fs.writeFileSync(
    path.join(root, "crates/koushi-core-testkit/Cargo.toml"),
    '[package]\nname = "koushi-core-testkit"\npublish = true\n  [dependencies.koushi-qa] # forbidden normal edge\npath = "../koushi-qa"\n[dev-dependencies]\nkoushi-core = { path = "../koushi-core" }\n'
  );
  fs.rmSync(path.join(root, "crates/koushi-core-testkit/tests/support"), {
    recursive: true,
    force: true
  });
  fs.rmSync(path.join(root, "crates/koushi-core-testkit/tests", testkitTargets[0]));
  write(
    root,
    path.join("crates/koushi-core/tests", testkitTargets[1]),
    "#[test]\nfn duplicate() {}\n"
  );
  write(
    root,
    path.join("crates/koushi-core-testkit/tests", coreLocalIntegrationTargets[0]),
    "#[test]\nfn misplaced() {}\n"
  );
  fs.writeFileSync(path.join(root, ".github/workflows/ci.yml"), "jobs: {}\n");
  fs.writeFileSync(
    path.join(root, "Cargo.toml"),
    '[workspace]\nmembers = ["crates/koushi-store", "crates/koushi-core", "crates/koushi-core-testkit", "crates/koushi-qa"]\ndefault-members = ["crates/koushi-store", "crates/koushi-core", "crates/koushi-core-testkit"]\n'
  );

  const violations = findLeafCrateBoundaryViolations(root);
  assert(violations.includes("koushi-core-testkit must not be a default workspace member"));
  assert(violations.includes("koushi-core-testkit must be publish-disabled"));
  assert(violations.includes("koushi-core-testkit must use dev-dependencies only"));
  assert(violations.includes("koushi-core-testkit must enable koushi-core/test-hooks"));
  assert(violations.includes("koushi-core must not self-depend for test hooks"));
  assert(violations.includes("koushi-core must not depend on koushi-core-testkit"));
  assert(violations.includes("koushi-qa must not depend on koushi-core-testkit"));
  assert(violations.includes("shared Core integration support missing from koushi-core-testkit"));
  assert(violations.includes(`koushi-core-testkit target missing: ${testkitTargets[0]}`));
  assert(violations.includes(`moved integration target remains in koushi-core: ${testkitTargets[1]}`));
  assert(
    violations.includes(
      `Core-local integration target moved unnecessarily: ${coreLocalIntegrationTargets[0]}`
    )
  );
  assert(violations.includes("unexpected Rust integration target set in koushi-core"));
  assert(violations.includes("unexpected Rust integration target set in koushi-core-testkit"));
  assert(violations.includes("CI must run koushi-core-testkit"));
});
