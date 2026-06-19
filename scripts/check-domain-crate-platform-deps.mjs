#!/usr/bin/env node
/**
 * check-domain-crate-platform-deps.mjs
 *
 * Verifies that the pure domain crate matrix-desktop-state does NOT directly
 * depend on platform/OS crates (keyring, windows-*, winapi, libc, tokio, etc.).
 *
 * Rule (from REPOSITORY_RULES.md and #87 Phase 0):
 *   matrix-desktop-state is the pure domain/serialization layer. It must
 *   remain free of platform OS crates so it can target WASM and future mobile.
 *
 * Grandfathered set (as of 2026-06-19, #87 Phase 0):
 *   matrix-desktop-core depends on keyring (apple-native, windows-native) —
 *   this is a known Phase 5 target for removal via SecretStore inversion.
 *   matrix-desktop-key also depends on keyring for the same reason.
 *   These are NOT checked here; only matrix-desktop-state's direct deps are.
 *
 * This script reads crates/matrix-desktop-state/Cargo.toml and fails if any
 * of the banned platform crate names appear in [dependencies].
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

const CARGO_TOML_PATH = join(
  repoRoot,
  "crates",
  "matrix-desktop-state",
  "Cargo.toml"
);

/**
 * Platform/OS crate names that must not appear as direct dependencies of
 * matrix-desktop-state. Extend this list when Phase 5 removes keyring from
 * core/key and any future platform dep is added elsewhere.
 *
 * Rationale per crate:
 *   keyring        — OS credential store (apple-native, windows-native); must
 *                    stay behind SecretStore port (tracked: #87 Phase 5).
 *   tokio          — async runtime; confined to executor.rs in core per
 *                    Platform Portability rule 2. State must be sync/pure.
 *   libc           — C FFI for OS calls; not allowed in pure domain layer.
 *   windows-*      — Windows-only OS bindings.
 *   winapi         — older Windows API binding.
 *   nix            — POSIX OS calls.
 *   arboard        — clipboard OS integration (#84).
 *   notify-rust    — OS notification integration (#10).
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

const cargoToml = readFileSync(CARGO_TOML_PATH, "utf-8");

// Simple TOML line-based parse: look for lines like `keyring = ...` or
// `keyring = { ... }` in the [dependencies] section.
const depSectionPattern = /^\[dependencies\]/m;
const nextSectionPattern = /^\[/m;

const depSectionMatch = depSectionPattern.exec(cargoToml);
if (!depSectionMatch) {
  console.log(
    "check-domain-crate-platform-deps: ok — no [dependencies] section found in matrix-desktop-state/Cargo.toml."
  );
  process.exit(0);
}

// Extract the text from [dependencies] to the next section (or end of file).
const afterDeps = cargoToml.slice(depSectionMatch.index + depSectionMatch[0].length);
const nextSectionMatch = nextSectionPattern.exec(afterDeps);
const depBlock = nextSectionMatch
  ? afterDeps.slice(0, nextSectionMatch.index)
  : afterDeps;

const violations = [];
for (const bannedDep of BANNED_PLATFORM_DEPS) {
  // Match `bannedDep =` or `bannedDep.workspace = true` at line start.
  const linePattern = new RegExp(`^\\s*${bannedDep}\\s*[=.]`, "m");
  if (linePattern.test(depBlock)) {
    violations.push(bannedDep);
  }
}

if (violations.length === 0) {
  console.log(
    "check-domain-crate-platform-deps: ok — matrix-desktop-state has no banned platform deps."
  );
  process.exit(0);
} else {
  console.error(
    "check-domain-crate-platform-deps: FAILED — matrix-desktop-state must not depend on platform/OS crates."
  );
  console.error(
    "This crate is the pure domain/serialization layer; it must target WASM and future mobile."
  );
  console.error(
    "See REPOSITORY_RULES.md and #87 Phase 5 for the SecretStore port plan.\n"
  );
  console.error("Banned deps found in [dependencies]:");
  for (const v of violations) {
    console.error(`  - ${v}`);
  }
  process.exit(1);
}
