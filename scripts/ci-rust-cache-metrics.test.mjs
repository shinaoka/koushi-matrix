import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

const script = fileURLToPath(new URL("./ci-rust-cache-metrics.mjs", import.meta.url));

test("Rust cache metrics records compilation, test, and target-size measurements", () => {
  const root = mkdtempSync(join(tmpdir(), "koushi-ci-metrics-"));
  const target = join(root, "target-ci");
  const log = join(root, "cargo.log");
  mkdirSync(target, { recursive: true });
  writeFileSync(join(target, "artifact.rlib"), "artifact");
  writeFileSync(
    log,
    [
      "   Compiling matrix-sdk v0.18.0",
      "    Checking koushi-core v0.1.0",
      "    Finished `ci` profile [unoptimized] target(s) in 42.00s",
      "test result: ok. 12 passed; 0 failed; 1 ignored"
    ].join("\n")
  );
  const result = spawnSync(process.execPath, [
    script,
    `--log=${log}`,
    `--target-dir=${target}`,
    "--mode=warm",
    "--cache-hit=true",
    "--elapsed-seconds=42"
  ], { encoding: "utf8" });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /rust_metric_compiling_lines=1/);
  assert.match(result.stdout, /rust_metric_checking_lines=1/);
  assert.match(result.stdout, /rust_metric_test_counts=\{"passed":12,"failed":0,"ignored":1\}/);
  assert.match(result.stdout, /rust_metric_target_bytes=8/);
});
