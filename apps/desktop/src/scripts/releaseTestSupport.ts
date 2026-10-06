import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

export const repoRoot = fileURLToPath(new URL("../../../../", import.meta.url));

export function runScript(script: string, args: string[] = []): string {
  return execFileSync(process.execPath, [script, ...args], { cwd: repoRoot, encoding: "utf8" });
}

const linuxProductionFiles = [
  "scripts/desktop-linux-gui-qa.mjs",
  "scripts/desktop-linux-gui-qa/options.mjs",
  "scripts/desktop-linux-gui-qa/main.mjs",
  "scripts/desktop-linux-gui-qa/registry.mjs",
  "scripts/desktop-linux-gui-qa/runtime.mjs",
  "scripts/desktop-linux-gui-qa/webdriver.mjs",
  "scripts/desktop-linux-gui-qa/evidence.mjs",
  "scripts/desktop-linux-gui-qa/redaction.mjs",
  "scripts/desktop-linux-gui-qa/local-session.mjs",
  "scripts/desktop-linux-gui-qa/scenarios/auth.mjs",
  "scripts/desktop-linux-gui-qa/scenarios/rooms-timeline.mjs",
  "scripts/desktop-linux-gui-qa/scenarios/media.mjs",
  "scripts/desktop-linux-gui-qa/scenarios/settings-security.mjs"
];

export function readLinuxProductionSource(): string {
  return linuxProductionFiles.map((file) => readFileSync(join(repoRoot, file), "utf8")).join("\n");
}

export function readRealHomeserverProductionSource(): string {
  const files = [
    "crates/koushi-qa/src/bin/real-homeserver-qa.rs",
    "crates/koushi-qa/src/bin/real_homeserver_qa/config.rs",
    "crates/koushi-qa/src/bin/real_homeserver_qa/credentials.rs",
    "crates/koushi-qa/src/bin/real_homeserver_qa/event_source.rs",
    "crates/koushi-qa/src/bin/real_homeserver_qa/waiters.rs",
    "crates/koushi-qa/src/bin/real_homeserver_qa/cleanup.rs",
    "crates/koushi-qa/src/bin/real_homeserver_qa/compat_flow.rs",
    "crates/koushi-qa/src/bin/real_homeserver_qa/startup_latency.rs"
  ];
  return files.map((file) => readFileSync(join(repoRoot, file), "utf8")).join("\n");
}

export function gitTrackedFiles(): string[] {
  return execFileSync("git", ["ls-files"], { cwd: repoRoot, encoding: "utf8" })
    .split("\n").map((file) => file.trim()).filter(Boolean);
}
