#!/usr/bin/env node
// Release credential-gate check (engineering-rules: Secrets rule 2).
//
// Verifies that debug/test-only credential injection paths are compiled out
// of release builds, two ways:
//
// 1. Structural: QA/debug env-var string literals in Rust sources may appear
//    only as const declarations directly under a
//    `#[cfg(any(debug_assertions, test))]` attribute. All other code must
//    reference the const, so an ungated reference fails the compile check.
// 2. Compile: `cargo check --release` on the app and key crates. Because the
//    consts vanish in release, any ungated reference is a hard compile error.
//
// Usage: node scripts/desktop-release-gate-check.mjs [--no-compile]

import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import path from "node:path";
import process from "node:process";

const repoRoot = path.resolve(path.dirname(new URL(import.meta.url).pathname), "..");
const noCompile = process.argv.includes("--no-compile");

// Credential-bearing env vars: these inject or redirect credentials and MUST
// be compiled out of release builds (engineering-rules: Secrets rule 2).
// QA behavior toggles that carry no credentials (MATRIX_DESKTOP_QA_TITLE,
// MATRIX_DESKTOP_SKIP_*) are intentionally not listed.
const GATED_ENV_LITERALS = [
  '"MATRIX_DESKTOP_QA_LOGIN_PIPE"',
  '"MATRIX_DESKTOP_QA_FILE_CREDENTIAL_STORE_DIR"',
];
const GATE_ATTR = /#\[cfg\((any\()?(debug_assertions|test)/;

const rsFiles = execFileSync("git", ["ls-files", "crates", "apps", "spikes"], {
  cwd: repoRoot,
  encoding: "utf8",
})
  .split("\n")
  .filter((f) => f.endsWith(".rs"));

const errors = [];
for (const file of rsFiles) {
  const lines = readFileSync(path.join(repoRoot, file), "utf8").split("\n");
  lines.forEach((line, i) => {
    if (!GATED_ENV_LITERALS.some((lit) => line.includes(lit))) return;
    const isConstDecl = /\bconst\b.*&str\b/.test(line);
    // The attribute must immediately precede the const declaration so a gate
    // on a neighboring item cannot satisfy this check.
    const window = [lines[i - 1] ?? "", line].join("\n");
    if (!isConstDecl || !GATE_ATTR.test(window)) {
      errors.push(
        `${file}:${i + 1}: QA/debug env literal must be a const declared under #[cfg(any(debug_assertions, test))]`,
      );
    }
  });
}

if (errors.length > 0) {
  console.error("release gate check FAILED (structural):");
  for (const e of errors) console.error(`  ${e}`);
  process.exit(1);
}
console.log("release gate check: structural ok");

if (!noCompile) {
  console.log("release gate check: cargo check --release (gated refs must not leak) ...");
  execFileSync("cargo", ["check", "--release", "--quiet", "-p", "matrix-desktop-key"], {
    cwd: repoRoot,
    stdio: "inherit",
  });
  // The Tauri app crate lives outside the workspace.
  execFileSync("cargo", ["check", "--release", "--quiet"], {
    cwd: path.join(repoRoot, "apps", "desktop", "src-tauri"),
    stdio: "inherit",
  });
  console.log("release gate check: compile ok");
}
