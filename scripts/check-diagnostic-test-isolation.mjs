#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(scriptDirectory, "..");
// Every crate whose tests can read the process-wide diagnostic ring. The
// diagnostics crate itself is excluded: its tests use private buffers.
const diagnosticTestRoots = fs
  .readdirSync(path.join(repositoryRoot, "crates"), { withFileTypes: true })
  .filter((entry) => entry.isDirectory() && entry.name !== "koushi-diagnostics")
  .flatMap((entry) => ["src", "tests"].map((dir) => path.join(repositoryRoot, "crates", entry.name, dir)))
  .filter((directory) => fs.existsSync(directory));

// Reads of the shared global ring. A cursor or detail window is a read too:
// it is only meaningful while no other reader can interleave its own setup.
const globalDiagnosticReads = [
  /\bkoushi_diagnostics::snapshot\(\)/u,
  /\btest_support::detail_snapshot\(\)/u,
  /\btest_support::detail_cursor\(\)/u,
  /\btest_support::detail_records_since\(/u,
  /\btest_support::rotation_snapshot\(\)/u
];
const sharedLocks = [/\btest_support::lock\(\)/u, /\btest_support::lock_async\(\)\.await/u];

export function findDiagnosticTestIsolationViolations(source, fileName) {
  const violations = [];
  const testAttribute = /#\[\s*(?:tokio::)?test(?:\([^\]]*\))?\s*\]/g;

  for (const attributeMatch of source.matchAll(testAttribute)) {
    const functionStart = source.indexOf("fn ", attributeMatch.index + attributeMatch[0].length);
    if (functionStart < 0) {
      continue;
    }
    const signatureEnd = source.indexOf("{", functionStart);
    if (signatureEnd < 0) {
      continue;
    }
    const functionName = source.slice(functionStart + 3, signatureEnd).match(/[A-Za-z0-9_]+/u)?.[0];
    if (!functionName) {
      continue;
    }
    const bodyEnd = matchingBrace(source, signatureEnd);
    if (bodyEnd < 0) {
      continue;
    }
    const body = stripRustStringsAndComments(source.slice(signatureEnd + 1, bodyEnd));
    if (
      globalDiagnosticReads.some((read) => read.test(body)) &&
      !sharedLocks.some((lock) => lock.test(body))
    ) {
      const line = source.slice(0, attributeMatch.index).split("\n").length;
      violations.push(`${fileName}:${line}:${functionName}`);
    }
  }

  return violations;
}

function stripRustStringsAndComments(source) {
  const characters = [...source];
  let quote = false;
  let escaped = false;
  let lineComment = false;
  let blockCommentDepth = 0;

  for (let index = 0; index < characters.length; index += 1) {
    const character = characters[index];
    const nextCharacter = characters[index + 1];

    if (lineComment) {
      characters[index] = " ";
      if (character === "\n") {
        lineComment = false;
      }
      continue;
    }
    if (blockCommentDepth > 0) {
      characters[index] = " ";
      if (character === "/" && nextCharacter === "*") {
        characters[index + 1] = " ";
        blockCommentDepth += 1;
        index += 1;
      } else if (character === "*" && nextCharacter === "/") {
        characters[index + 1] = " ";
        blockCommentDepth -= 1;
        index += 1;
      }
      continue;
    }
    if (quote) {
      characters[index] = " ";
      if (escaped) {
        escaped = false;
      } else if (character === "\\") {
        escaped = true;
      } else if (character === '"') {
        quote = false;
      }
      continue;
    }
    if (character === "/" && nextCharacter === "/") {
      characters[index] = " ";
      characters[index + 1] = " ";
      lineComment = true;
      index += 1;
      continue;
    }
    if (character === "/" && nextCharacter === "*") {
      characters[index] = " ";
      characters[index + 1] = " ";
      blockCommentDepth = 1;
      index += 1;
      continue;
    }
    if (character === '"') {
      characters[index] = " ";
      quote = true;
    }
  }

  return characters.join("");
}

function matchingBrace(source, openingBrace) {
  let depth = 0;
  let quote = null;
  let escaped = false;
  let lineComment = false;
  let blockCommentDepth = 0;

  for (let index = openingBrace; index < source.length; index += 1) {
    const character = source[index];
    const nextCharacter = source[index + 1];

    if (lineComment) {
      if (character === "\n") {
        lineComment = false;
      }
      continue;
    }
    if (blockCommentDepth > 0) {
      if (character === "/" && nextCharacter === "*") {
        blockCommentDepth += 1;
        index += 1;
      } else if (character === "*" && nextCharacter === "/") {
        blockCommentDepth -= 1;
        index += 1;
      }
      continue;
    }
    if (quote) {
      if (escaped) {
        escaped = false;
      } else if (character === "\\") {
        escaped = true;
      } else if (character === quote) {
        quote = null;
      }
      continue;
    }
    if (character === "/" && nextCharacter === "/") {
      lineComment = true;
      index += 1;
      continue;
    }
    if (character === "/" && nextCharacter === "*") {
      blockCommentDepth = 1;
      index += 1;
      continue;
    }
    if (character === '"' || character === "'") {
      quote = character;
      continue;
    }
    if (character === "{") {
      depth += 1;
    } else if (character === "}") {
      depth -= 1;
      if (depth === 0) {
        return index;
      }
    }
  }

  return -1;
}

function rustFiles(directory) {
  return fs
    .readdirSync(directory, { withFileTypes: true })
    .flatMap((entry) => {
      const entryPath = path.join(directory, entry.name);
      if (entry.isDirectory()) {
        return rustFiles(entryPath);
      }
      return entry.isFile() && entry.name.endsWith(".rs") ? [entryPath] : [];
    });
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const violations = diagnosticTestRoots.flatMap((root) =>
    rustFiles(root).flatMap((filePath) =>
      findDiagnosticTestIsolationViolations(
        fs.readFileSync(filePath, "utf8"),
        path.relative(repositoryRoot, filePath)
      )
    )
  );

  if (violations.length > 0) {
    console.error("Diagnostic snapshot tests must hold the process-wide test lock:");
    for (const violation of violations) {
      console.error(`- ${violation}`);
    }
    process.exitCode = 1;
  }
}
