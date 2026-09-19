#!/usr/bin/env node
import { appendFileSync, existsSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

const logPath = resolve(requiredOption("--log"));
const targetDir = resolve(optionValue("--target-dir") ?? "target");
const mode = optionValue("--mode") ?? "unknown";
const cacheHit = optionValue("--cache-hit") ?? "false";
const elapsedSeconds = Number(optionValue("--elapsed-seconds") ?? "0");
const outputPath = optionValue("--output");
const log = readFileSync(logPath, "utf8");

const compiling = countLines(log, /^\s*Compiling\s+/gmu);
const checking = countLines(log, /^\s*Checking\s+/gmu);
const finished = countLines(log, /^\s*Finished\s+/gmu);
const testCounts = [...log.matchAll(/test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored/gu)].reduce(
  (totals, match) => ({
    passed: totals.passed + Number(match[1]),
    failed: totals.failed + Number(match[2]),
    ignored: totals.ignored + Number(match[3])
  }),
  { passed: 0, failed: 0, ignored: 0 }
);
const record = {
  mode,
  cache_hit: cacheHit === "true",
  elapsed_seconds: elapsedSeconds,
  compiling_lines: compiling,
  checking_lines: checking,
  finished_lines: finished,
  test_counts: testCounts,
  target_bytes: directoryBytes(targetDir),
  log_bytes: statSync(logPath).size
};

for (const [key, value] of Object.entries(record)) {
  console.log(`rust_metric_${key}=${typeof value === "object" ? JSON.stringify(value) : value}`);
}
if (outputPath) writeFileSync(resolve(outputPath), `${JSON.stringify(record, null, 2)}\n`);

const summaryPath = process.env.GITHUB_STEP_SUMMARY;
if (summaryPath) {
  appendFileSync(
    summaryPath,
    [
      `### Rust cache benchmark: ${mode}`,
      "",
      `- cache hit: \`${record.cache_hit}\``,
      `- elapsed: ${elapsedSeconds} s`,
      `- Compiling lines: ${compiling}`,
      `- Checking lines: ${checking}`,
      `- Finished lines: ${finished}`,
      `- tests: ${testCounts.passed} passed, ${testCounts.failed} failed, ${testCounts.ignored} ignored`,
      `- target size: ${record.target_bytes} bytes`,
      `- cargo log size: ${record.log_bytes} bytes`,
      ""
    ].join("\n")
  );
}

function requiredOption(name) {
  const value = optionValue(name);
  if (!value) throw new Error(`${name}= is required`);
  return value;
}

function optionValue(name) {
  const prefix = `${name}=`;
  const inline = process.argv.slice(2).find((arg) => arg.startsWith(prefix));
  return inline ? inline.slice(prefix.length) : undefined;
}

function countLines(value, pattern) {
  return [...value.matchAll(pattern)].length;
}

function directoryBytes(path) {
  if (!existsSync(path)) return 0;
  let bytes = 0;
  for (const entry of readdirSync(path, { withFileTypes: true })) {
    const child = join(path, entry.name);
    bytes += entry.isDirectory() ? directoryBytes(child) : statSync(child).size;
  }
  return bytes;
}
