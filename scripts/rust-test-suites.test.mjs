import assert from "node:assert/strict";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const layouts = [
  ["koushi-state", ["state"], []],
  ["koushi-sdk", ["sdk"], []],
  ["koushi-core", ["core"], []],
  ["koushi-core-testkit", ["contracts", "runtime"], ["runtime_navigation", "stack_budget"]],
  ["koushi-mcp-host", ["bridge"], []],
];

function checkSuiteModules(directory) {
  const source = readFileSync(join(directory, "main.rs"), "utf8");
  const declared = [...source.matchAll(/^mod ([a-z0-9_]+);$/gm)].map((match) => match[1]);
  const files = readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    if (entry.isFile() && entry.name.endsWith(".rs") && entry.name !== "main.rs") {
      return [entry.name.slice(0, -3)];
    }
    return entry.isDirectory() && existsSync(join(directory, entry.name, "mod.rs"))
      ? [entry.name] : [];
  });
  assert.deepEqual(declared.sort(), files.sort(), "module inventory");
  assert.doesNotMatch(source, /#\[(?:tokio::)?test\b/);
}

for (const [crate, suites, standalone] of layouts) {
  test(`${crate} has only canon suite targets and declares every feature module`, () => {
    const tests = join(root, "crates", crate, "tests");
    const targets = readdirSync(tests, { withFileTypes: true }).flatMap((entry) => {
      if (entry.isFile() && entry.name.endsWith(".rs")) return [entry.name.slice(0, -3)];
      return entry.isDirectory() && existsSync(join(tests, entry.name, "main.rs"))
        ? [entry.name] : [];
    });
    assert.deepEqual(targets.sort(), [...suites, ...standalone].sort());
    for (const suite of suites) checkSuiteModules(join(tests, suite));
  });
}

test("suite inventory rejects an undeclared feature/mod.rs", () => {
  const directory = mkdtempSync(join(tmpdir(), "koushi-suite-inventory-"));
  try {
    mkdirSync(join(directory, "feature"));
    writeFileSync(join(directory, "feature/mod.rs"), "#[test]\nfn case() {}\n");
    writeFileSync(join(directory, "main.rs"), "");
    assert.throws(() => checkSuiteModules(directory), assert.AssertionError);
    writeFileSync(join(directory, "main.rs"), "mod feature;\n");
    assert.doesNotThrow(() => checkSuiteModules(directory));
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

test("ordinary Rust CI runs exactly one feature-unified workspace test command", () => {
  const workflow = readFileSync(join(root, ".github/workflows/ci.yml"), "utf8");
  const rustJob = workflow.match(/\n  rust:\n([\s\S]*?)(?=\n  [a-z0-9_-]+:\n)/)?.[1];
  assert.ok(rustJob, "Rust CI job exists");
  const commands = [...rustJob.matchAll(/^\s*(?:run: )?(cargo test [^\n]+)/gm)]
    .map((match) => match[1]);
  assert.equal(commands.length, 1, "per-package follow-up proofs rebuild an already tested graph");
  assert.match(commands[0], /--profile ci --workspace --exclude sidebar-composition --exclude key-management/);
  assert.doesNotMatch(commands[0], /--(?:test|lib|no-run)\b/);
  assert.match(rustJob, /node --test[^\n]*scripts\/rust-test-suites\.test\.mjs/);
});
