import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const SDK_SUBMODULE_PATH = "vendor/matrix-rust-sdk";
const SDK_DEPENDENCY_PATHS = new Map([
  ["matrix-sdk", "vendor/matrix-rust-sdk/crates/matrix-sdk"],
  ["matrix-sdk-base", "vendor/matrix-rust-sdk/crates/matrix-sdk-base"],
  ["matrix-sdk-search", "vendor/matrix-rust-sdk/crates/matrix-sdk-search"],
  ["matrix-sdk-test", "vendor/matrix-rust-sdk/testing/matrix-sdk-test"],
  ["matrix-sdk-ui", "vendor/matrix-rust-sdk/crates/matrix-sdk-ui"],
]);
const SDK_PATH_DIAGNOSTIC =
  "Matrix SDK workspace dependencies must resolve from vendor Matrix SDK submodule paths";

export function parseSubmoduleStatus(output) {
  const line = output
    .split(/\r?\n/)
    .map((entry) => entry.trimEnd())
    .find((entry) => entry.includes(SDK_SUBMODULE_PATH));

  if (!line) {
    return { ok: false, state: "missing", path: SDK_SUBMODULE_PATH };
  }

  const marker = line[0];
  const revision = line.slice(1).trimStart().split(/\s+/, 1)[0] ?? "";
  if (marker === " ") {
    return { ok: true, state: "synced", path: SDK_SUBMODULE_PATH, revision };
  }
  if (marker === "-") {
    return { ok: false, state: "uninitialized", path: SDK_SUBMODULE_PATH, revision };
  }
  if (marker === "+") {
    return { ok: false, state: "stale", path: SDK_SUBMODULE_PATH, revision };
  }
  if (marker === "U") {
    return { ok: false, state: "conflicted", path: SDK_SUBMODULE_PATH, revision };
  }
  return { ok: false, state: "unknown", path: SDK_SUBMODULE_PATH, revision };
}

export function readSubmoduleStatus({ repoRoot, fixturePath } = {}) {
  if (fixturePath) {
    return readFileSync(fixturePath, "utf8");
  }

  const result = spawnSync(
    "git",
    ["submodule", "status", "--recursive", SDK_SUBMODULE_PATH],
    { cwd: repoRoot, encoding: "utf8" },
  );
  if (result.status !== 0) {
    return "";
  }
  return result.stdout;
}

export function assertSdkWorkspaceUsesSubmodulePaths({ repoRoot, manifestPath } = {}) {
  const manifest = readFileSync(manifestPath ?? join(repoRoot, "Cargo.toml"), "utf8");
  for (const [name, expectedPath] of SDK_DEPENDENCY_PATHS) {
    const escapedName = name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    const declarations = [
      ...manifest.matchAll(new RegExp(`^${escapedName}\\s*=\\s*\\{([^}]*)\\}`, "gm")),
    ];
    const body = declarations[0]?.[1] ?? "";
    const path = /(?:^|,)\s*path\s*=\s*"([^"]+)"/.exec(body)?.[1];
    const hasForbiddenSource = /(?:^|,)\s*(?:git|rev)\s*=/.test(body);
    if (declarations.length !== 1 || path !== expectedPath || hasForbiddenSource) {
      throw new Error(SDK_PATH_DIAGNOSTIC);
    }
  }
}

export function assertSdkSubmoduleSynced({ repoRoot, fixturePath, manifestPath } = {}) {
  assertSdkWorkspaceUsesSubmodulePaths({ repoRoot, manifestPath });
  const status = parseSubmoduleStatus(readSubmoduleStatus({ repoRoot, fixturePath }));
  if (!status.ok) {
    throw new Error(statusDiagnostic(status.state));
  }
  return status;
}

function statusDiagnostic(state) {
  const detailByState = {
    conflicted: "has merge conflicts",
    missing: "is missing from git submodule status",
    stale: "is stale",
    uninitialized: "is not initialized",
    unknown: "has an unknown status",
  };
  const detail = detailByState[state] ?? detailByState.unknown;
  return `vendor Matrix SDK submodule ${detail}. Run: git submodule update --init --recursive ${SDK_SUBMODULE_PATH}`;
}
