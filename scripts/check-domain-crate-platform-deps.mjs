#!/usr/bin/env node
/**
 * check-domain-crate-platform-deps.mjs
 *
 * Verifies that the pure domain crate koushi-state does NOT directly
 * depend on platform/OS crates (keyring, windows-*, winapi, libc, tokio, etc.)
 * in ANY dependency table.
 *
 * Rule (from REPOSITORY_RULES.md and #87 Phase 0):
 *   koushi-state is the pure domain/serialization layer. It must
 *   remain free of platform OS crates so it can target WASM and future mobile.
 *
 * Grandfathered set (as of 2026-06-19, #87 Phase 0):
 *   koushi-core depends on keyring (apple-native, windows-native) —
 *   this is a known Phase 5 target for removal via SecretStore inversion.
 *   koushi-key also depends on keyring for the same reason.
 *   These are NOT checked here; only koushi-state's deps are.
 *
 * Coverage: ALL dependency tables are scanned to prevent a platform crate from
 * sneaking in through a target-specific or build dep:
 *   [dependencies]
 *   [dev-dependencies]
 *   [build-dependencies]
 *   [target.'cfg(...)'.dependencies]
 *   [target.'cfg(...)'.dev-dependencies]
 *   [target.'cfg(...)'.build-dependencies]
 *
 * Additionally, renamed packages are detected via the `package = "..."` key
 * so that `foo = { package = "keyring" }` is caught even though the dep is
 * declared under the alias `foo`.
 *
 * Usage:
 *   node scripts/check-domain-crate-platform-deps.mjs
 *
 * Exit 0 if clean; exit 1 with details on violation.
 */

import { readFileSync } from "fs";
import { join } from "path";
import { fileURLToPath } from "url";

const __dirname = fileURLToPath(new URL(".", import.meta.url));
const repoRoot = join(__dirname, "..");

/**
 * Platform/OS crate names that must not appear as direct dependencies of the
 * pure domain crates. koushi-state forbids the full list (incl. tokio);
 * koushi-core forbids the same list EXCEPT tokio, which it confines to
 * executor.rs per Platform Portability rule 2. #87 Phase 5 removed keyring from
 * core AND key: credential errors are abstracted into CredentialBackendErrorKind
 * (koushi-key is a pure port) and the keyring adapter now lives only in the
 * apps/desktop/src-tauri platform binary, so all three domain crates are
 * enforced keyring-free here.
 *
 * Rationale per crate:
 *   keyring        — OS credential store (apple-native, windows-native); lives
 *                    only in the src-tauri platform binary, injected via the
 *                    CredentialBackend port (#87 Phase 5).
 *   tokio          — async runtime; confined to executor.rs in core per
 *                    Platform Portability rule 2. State must be sync/pure.
 *   libc           — C FFI for OS calls; not allowed in pure domain layer.
 *   windows-sys    — Windows-only OS bindings.
 *   windows-core   — Windows-only OS bindings.
 *   windows-targets — Windows-only OS bindings.
 *   winapi         — older Windows API binding.
 *   nix            — POSIX OS calls.
 *   arboard        — clipboard OS integration (#84).
 *   notify-rust    — OS notification integration (#10).
 *   core-foundation — macOS/iOS OS bindings.
 *   security-framework — macOS/iOS OS bindings.
 */
const BANNED_PLATFORM_DEPS = [
  "keyring",
  "tokio",
  "libc",
  "windows-sys",
  "windows-core",
  "windows-targets",
  "winapi",
  "nix",
  "arboard",
  "notify-rust",
  "core-foundation",
  "security-framework",
];

// Pure domain crates and the platform deps each forbids. State is sync/pure and
// forbids everything; core may use tokio (executor.rs only) but nothing else.
// koushi-key is now a pure port crate (keyring-free) and forbids the
// full list including keyring.
const DOMAIN_CRATES = [
  { name: "koushi-state", banned: BANNED_PLATFORM_DEPS },
  {
    name: "koushi-core",
    banned: BANNED_PLATFORM_DEPS.filter((dep) => dep !== "tokio"),
  },
  { name: "koushi-key", banned: BANNED_PLATFORM_DEPS },
];

/**
 * Determine if a line is a dependency section header.
 *
 * Recognised forms:
 *   [dependencies]
 *   [dev-dependencies]
 *   [build-dependencies]
 *   [target.'cfg(...)'.dependencies]      (single-quoted)
 *   [target."cfg(...)".dependencies]      (double-quoted)
 *   [target.'cfg(...)'.dev-dependencies]
 *   [target.'cfg(...)'.build-dependencies]
 */
function isDepSectionHeader(line) {
  const trimmed = line.trim();
  if (!trimmed.startsWith("[") || trimmed.startsWith("[[")) return false;
  const inner = trimmed.replace(/^\[/, "").replace(/]$/, "").trim();
  if (
    inner === "dependencies" ||
    inner === "dev-dependencies" ||
    inner === "build-dependencies"
  ) {
    return true;
  }
  // target-conditional tables
  if (/^target\s*\./.test(inner)) {
    return /\.(dependencies|dev-dependencies|build-dependencies)$/.test(inner);
  }
  return false;
}

function isAnySectionHeader(line) {
  const trimmed = line.trim();
  return trimmed.startsWith("[") && !trimmed.startsWith("[[");
}

// Scan one crate's Cargo.toml for any banned platform dep (declared name or
// `package = "..."` alias) across all dependency tables.
function scanCrate(crateName, banned) {
  const cargoToml = readFileSync(
    join(repoRoot, "crates", crateName, "Cargo.toml"),
    "utf-8"
  );
  const violations = [];
  let inDepSection = false;
  for (const line of cargoToml.split("\n")) {
    const trimmed = line.trim();
    if (trimmed === "" || trimmed.startsWith("#")) continue;
    if (isAnySectionHeader(line)) {
      inDepSection = isDepSectionHeader(line);
      continue;
    }
    if (!inDepSection) continue;

    const nameMatch = /^([A-Za-z0-9_-]+)\s*[=.]/.exec(trimmed);
    if (nameMatch && banned.includes(nameMatch[1])) {
      violations.push({ kind: "name", dep: nameMatch[1], line: trimmed });
    }
    const packageMatch = /package\s*=\s*["']([A-Za-z0-9_-]+)["']/.exec(trimmed);
    if (packageMatch && banned.includes(packageMatch[1])) {
      violations.push({ kind: "package-alias", dep: packageMatch[1], line: trimmed });
    }
  }
  return violations;
}

let failed = false;
for (const crate of DOMAIN_CRATES) {
  const violations = scanCrate(crate.name, crate.banned);
  if (violations.length === 0) {
    console.log(
      `check-domain-crate-platform-deps: ok — ${crate.name} has no banned platform deps.`
    );
    continue;
  }
  failed = true;
  console.error(
    `check-domain-crate-platform-deps: FAILED — ${crate.name} must not depend on platform/OS crates.`
  );
  console.error(
    "Pure domain crates must target WASM and future mobile; platform access goes behind a port."
  );
  console.error(
    "See REPOSITORY_RULES.md and #87 Phase 5 (keyring lives only in the src-tauri platform binary, behind the CredentialBackend port).\n"
  );
  console.error(`Banned deps found in ${crate.name} (all dependency tables scanned):`);
  for (const v of violations) {
    const tag = v.kind === "package-alias" ? " (via package alias)" : "";
    console.error(`  - ${v.dep}${tag}: ${v.line}`);
  }
}

process.exit(failed ? 1 : 0);
