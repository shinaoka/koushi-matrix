#!/usr/bin/env node
/**
 * check-tauri-adapter-boundary.mjs
 *
 * Verifies that apps/desktop/src-tauri/src/** does NOT directly call or import
 * matrix_desktop_sdk wrapper APIs. The Tauri adapter is a transport layer: it
 * routes CoreCommand/CoreEvent through CoreRuntime and must not reach the SDK
 * wrapper directly.
 *
 * Rule (from REPOSITORY_RULES.md "Architecture And Ownership"):
 *   "apps/desktop/src-tauri is a transport adapter. It holds CoreRuntime,
 *    sends commands, forwards events/snapshots, and does not call Matrix SDK
 *    wrapper APIs directly."
 *
 * What "matrix_desktop_sdk" means in this context: the crate at
 * crates/matrix-desktop-sdk, which is the low-level SDK adapter. The adapter
 * layer (src-tauri) must not call functions on that crate directly; all SDK
 * operations must go through CoreRuntime/CoreCommand in matrix-desktop-core.
 *
 * Usage:
 *   node scripts/check-tauri-adapter-boundary.mjs
 *
 * Exit 0 if no violation found; exit 1 with details if any file matches.
 */

import { readFileSync, readdirSync, statSync } from "fs";
import { join, relative } from "path";
import { fileURLToPath } from "url";

const __dirname = fileURLToPath(new URL(".", import.meta.url));
const repoRoot = join(__dirname, "..");
const tauriSrcDir = join(repoRoot, "apps", "desktop", "src-tauri", "src");

/**
 * Patterns that indicate a direct matrix_desktop_sdk reference.
 *
 * We catch all of:
 *   `use matrix_desktop_sdk::...`     — importing SDK types into scope
 *   `matrix_desktop_sdk::...`         — direct qualified path / call
 *   `use matrix_desktop_sdk as ...`   — aliased import
 *   `extern crate matrix_desktop_sdk` — explicit extern crate declaration
 *   `matrix_desktop_sdk` as a bare token in a use/path position
 *
 * A single regex covers all forms by matching the bare identifier token
 * `matrix_desktop_sdk` on any non-comment line.  The comment filter below
 * excludes lines that are pure `//` comments (e.g., enforcement reminder
 * in commands.rs: "No `matrix_desktop_sdk` calls").
 */
const VIOLATION_PATTERN = /\bmatrix_desktop_sdk\b/;

/**
 * Skip pure line comments (`//...`). Block comments (slash-star ... star-slash)
 * spanning multiple lines are not filtered — a real use inside a block comment
 * would be unusual and worth flagging for manual review.
 */
const COMMENT_LINE_PATTERN = /^\s*\/\//;

/**
 * Recursively collect all .rs files under a directory.
 */
function collectRsFiles(dir) {
  const files = [];
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    const st = statSync(full);
    if (st.isDirectory()) {
      files.push(...collectRsFiles(full));
    } else if (entry.endsWith(".rs")) {
      files.push(full);
    }
  }
  return files;
}

const rsFiles = collectRsFiles(tauriSrcDir);
const violations = [];

for (const filePath of rsFiles) {
  const content = readFileSync(filePath, "utf-8");
  const lines = content.split("\n");
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (COMMENT_LINE_PATTERN.test(line)) {
      // Skip comment-only lines (docstrings, enforcement reminders).
      continue;
    }
    if (VIOLATION_PATTERN.test(line)) {
      violations.push({
        file: relative(repoRoot, filePath),
        line: i + 1,
        text: line.trimEnd(),
      });
    }
  }
}

if (violations.length === 0) {
  console.log(
    "check-tauri-adapter-boundary: ok — src-tauri does not call matrix_desktop_sdk directly."
  );
  process.exit(0);
} else {
  console.error(
    "check-tauri-adapter-boundary: FAILED — src-tauri must not call matrix_desktop_sdk wrapper APIs directly."
  );
  console.error(
    "The Tauri adapter routes operations through CoreRuntime/CoreCommand (matrix-desktop-core)."
  );
  console.error("See REPOSITORY_RULES.md 'Architecture And Ownership'.\n");
  for (const v of violations) {
    console.error(`  ${v.file}:${v.line}: ${v.text}`);
  }
  process.exit(1);
}
