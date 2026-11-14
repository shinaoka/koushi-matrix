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

test("the macOS lane runs the relaunch and restart-barrier tests", () => {
  // #1896: the Dock fix's own macOS-only behaviour was gated by tests no lane
  // ran. The step, its vacuous-run guard and the module paths its filters name
  // are one contract: libtest exits green when a filter selects nothing.
  const workflow = readFileSync(join(root, ".github/workflows/ci.yml"), "utf8");
  const macosJob = workflow.match(/\n  macos-cargo-check:\n([\s\S]*?)(?=\n  [a-z0-9_-]+:\n)/)?.[1];
  assert.ok(macosJob, "macOS CI job exists");
  assert.match(
    macosJob,
    /name: macOS relaunch handoff tests[\s\S]*cargo test --profile ci -p koushi-desktop --lib --locked -- relaunch:: restart_barrier_tests::/
  );
  assert.match(macosJob, /grep -Eq '\^test result: ok\\\. \(\[1-9\]\[0-9\]\+\) passed'/);
  const lib = readFileSync(join(root, "apps/desktop/src-tauri/src/lib.rs"), "utf8");
  const relaunch = readFileSync(join(root, "apps/desktop/src-tauri/src/relaunch.rs"), "utf8");
  assert.match(lib, /#\[cfg\(any\(target_os = "macos", test\)\)\]\nmod relaunch;/);
  assert.match(relaunch, /#\[path = "relaunch_tests\.rs"\]\nmod relaunch_tests;/);
  assert.match(lib, /#\[path = "app_updates\/restart_barrier_tests\.rs"\]\nmod restart_barrier_tests;/);
});

test("production still receives the relaunch seams the tests exercise", () => {
  // The macOS-only tests call the injected seams, because running the real
  // `/usr/bin/open` or `/usr/bin/logger` is not a test. That leaves the
  // production one-liners unexecuted anywhere, so pin them here: whichever
  // constant or function the wrapper hands over is what production runs.
  const relaunch = readFileSync(join(root, "apps/desktop/src-tauri/src/relaunch.rs"), "utf8");
  assert.match(relaunch, /const LAUNCH_SERVICES_OPEN: &str = "\/usr\/bin\/open";/);
  assert.match(
    relaunch,
    /fn launch_bundle\(plan: &RelaunchPlan\) -> bool \{\n\s+launch_bundle_with\(Path::new\(LAUNCH_SERVICES_OPEN\), plan\)\n\}/
  );
  assert.match(
    relaunch,
    /fn run_helper\(arguments: &\[OsString\]\) -> i32 \{\n\s+run_helper_with\(arguments, RELAUNCH_WAIT, launch_bundle, log_failure_line\)\n\}/
  );
  assert.match(
    relaunch,
    /Command::new\("\/usr\/bin\/logger"\)\s*\.arg\("-t"\)\s*\.arg\("koushi"\)/
  );
});
