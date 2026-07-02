import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const SDK_SUBMODULE_PATH = "vendor/matrix-rust-sdk";
const SDK_GIT_URL = "https://github.com/shinaoka/matrix-rust-sdk-work.git";

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

export function readPinnedSdkRevision({ repoRoot }) {
  const manifest = readFileSync(join(repoRoot, "Cargo.toml"), "utf8");
  const escapedUrl = SDK_GIT_URL.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const dependencyPattern = new RegExp(
    `^matrix-sdk(?:-base|-test|-ui)?\\s*=\\s*\\{[^}]*git\\s*=\\s*"${escapedUrl}"[^}]*rev\\s*=\\s*"([0-9a-f]{40})"`,
    "gm",
  );
  const revisions = [...manifest.matchAll(dependencyPattern)].map((match) => match[1]);
  const uniqueRevisions = new Set(revisions);
  if (revisions.length < 4 || uniqueRevisions.size !== 1) {
    throw new Error("vendor Matrix SDK pinned revision is missing or inconsistent in Cargo.toml");
  }
  return revisions[0];
}

export function assertSdkSubmoduleSynced({ repoRoot, fixturePath, expectedRevision } = {}) {
  const status = parseSubmoduleStatus(readSubmoduleStatus({ repoRoot, fixturePath }));
  if (!status.ok) {
    throw new Error(statusDiagnostic(status.state));
  }

  const pinnedRevision = expectedRevision ?? readPinnedSdkRevision({ repoRoot });
  if (status.revision !== pinnedRevision) {
    throw new Error(statusDiagnostic("mismatched"));
  }
  return status;
}

function statusDiagnostic(state) {
  if (state === "mismatched") {
    return (
      "vendor Matrix SDK submodule does not match the pinned SDK revision. " +
      `Update the root SDK rev and submodule gitlink together, then run: git submodule update --init --recursive ${SDK_SUBMODULE_PATH}`
    );
  }

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
