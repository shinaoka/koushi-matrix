// Bundles the `koushi-mcp` launcher and read-only `koushi` CLI with the desktop app (#1536).
//
// Tauri copies `bundle.externalBin` entries during the desktop crate's
// build script, and fails when the staged file is missing. The entry is
// therefore passed only through the release build scripts' `--config`, never
// in tauri.conf.json, so `cargo build`/`cargo test` of koushi-desktop and CI
// jobs that never bundle are unaffected.

import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, statSync } from "node:fs";
import { join } from "node:path";

/** Tauri resolves this to `binaries/koushi-mcp-<target-triple>[.exe]`. */
export const MCP_SIDECAR_CONFIG = { bundle: { externalBin: ["binaries/koushi-mcp", "binaries/koushi"] } };

export const BRIDGE_BINARIES = ["koushi-mcp", "koushi"];

export function stagedSidecarPath(repoRoot, target, binary = "koushi-mcp") {
  const suffix = target.includes("windows") ? ".exe" : "";
  return join(repoRoot, "apps", "desktop", "src-tauri", "binaries", `${binary}-${target}${suffix}`);
}

/** The Rust host triple, used when a build script passes no `--target`. */
export function hostTarget() {
  const result = spawnSync("rustc", ["-vV"], { encoding: "utf8" });
  const host = result.stdout?.match(/^host: (\S+)$/m)?.[1];
  if (result.status !== 0 || !host) {
    throw new Error("could not determine the Rust host target (rustc -vV)");
  }
  return host;
}

/**
 * Builds the launcher in release mode for `target` and stages it where Tauri
 * expects it. Returns the staged path.
 */
export function stageMcpSidecar({ repoRoot, target, env = process.env, shell = false }) {
  const result = spawnSync(
    "cargo",
    ["build", "--release", "--locked", "-p", "koushi-mcp", "--bins", "--target", target],
    { cwd: repoRoot, stdio: "inherit", env, shell }
  );
  if (result.error || result.status !== 0) {
    throw new Error(`building koushi-mcp for ${target} failed`);
  }
  const suffix = target.includes("windows") ? ".exe" : "";
  for (const binary of BRIDGE_BINARIES) {
    const built = join(repoRoot, "target", target, "release", `${binary}${suffix}`);
    const staged = stagedSidecarPath(repoRoot, target, binary);
    mkdirSync(join(staged, ".."), { recursive: true });
    copyFileSync(built, staged);
  }
  const staged = stagedSidecarPath(repoRoot, target);
  return staged;
}

/** Fails unless the staged launcher exists and is non-empty (bundle-only runs). */
export function requireStagedSidecar(repoRoot, target) {
  for (const binary of BRIDGE_BINARIES) {
    const staged = stagedSidecarPath(repoRoot, target, binary);
    if (!existsSync(staged) || statSync(staged).size === 0) {
      throw new Error(`the ${binary} executable is not staged at ${staged}; run the build stage first`);
    }
  }
  const staged = stagedSidecarPath(repoRoot, target);
  return staged;
}

/** Adds the sidecar entry to a Tauri config fragment without dropping keys. */
export function withMcpSidecar(config = {}) {
  return {
    ...config,
    bundle: { ...(config.bundle ?? {}), ...MCP_SIDECAR_CONFIG.bundle }
  };
}
