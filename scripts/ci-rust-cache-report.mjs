#!/usr/bin/env node
import { appendFileSync, existsSync, readdirSync, statSync } from "node:fs";
import { join, relative, resolve } from "node:path";

const targetDir = resolve(optionValue("--target-dir") ?? "target");
const profile = optionValue("--profile") ?? "dev";
const sdkCacheHit = optionValue("--sdk-cache-hit") ?? "";
const label = optionValue("--label") ?? "rust";

const profileDir = join(targetDir, profile === "dev" ? "debug" : profile === "release" ? "release" : profile);
const sdkFingerprintCount = countMatchingEntries(join(profileDir, ".fingerprint"), (name) => name.startsWith("matrix-sdk-"));
const sdkArtifactCount = countMatchingEntries(join(profileDir, "deps"), (name) =>
  /^libmatrix_sdk(?:_|\.)|^matrix_sdk.*\.d$/u.test(name)
);
const targetBytes = directoryBytes(targetDir);

console.log(`rust_cache_label=${label}`);
console.log(`rust_cache_target_present=${existsSync(targetDir)}`);
console.log(`rust_cache_profile=${profile}`);
console.log(`rust_cache_profile_present=${existsSync(profileDir)}`);
console.log(`rust_cache_sdk_cache_hit=${sdkCacheHit || "false"}`);
console.log(`rust_cache_sdk_fingerprints=${sdkFingerprintCount}`);
console.log(`rust_cache_sdk_artifacts=${sdkArtifactCount}`);
console.log(`rust_cache_target_bytes=${targetBytes}`);

if (sdkCacheHit === "true" && (sdkFingerprintCount === 0 || sdkArtifactCount === 0)) {
  throw new Error("vendored Matrix SDK cache reported a hit but no CI-profile artifacts were restored");
}

const summaryPath = process.env.GITHUB_STEP_SUMMARY;
if (summaryPath) {
  const targetLabel = relative(process.env.GITHUB_WORKSPACE ?? process.cwd(), targetDir) || ".";
  const summary = [
    `### Rust cache report: ${label}`,
    "",
    `- target: \`${targetLabel}\``,
    `- profile: \`${profile}\``,
    `- SDK cache hit: \`${sdkCacheHit || "false"}\``,
    `- SDK fingerprints: ${sdkFingerprintCount}`,
    `- SDK artifacts: ${sdkArtifactCount}`,
    `- target size: ${targetBytes} bytes`,
    ""
  ].join("\n");
  appendFileSync(summaryPath, summary);
}

function optionValue(name) {
  const prefix = `${name}=`;
  const inline = process.argv.slice(2).find((arg) => arg.startsWith(prefix));
  return inline ? inline.slice(prefix.length) : undefined;
}

function countMatchingEntries(directory, predicate) {
  if (!existsSync(directory)) return 0;
  let count = 0;
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (predicate(entry.name)) count += 1;
    if (entry.isDirectory()) count += countMatchingEntries(path, predicate);
  }
  return count;
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
