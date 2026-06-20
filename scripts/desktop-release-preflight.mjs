#!/usr/bin/env node
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const strictSigning = process.argv.includes("--strict-signing");
const checkConfig = process.argv.includes("--check-config") || strictSigning;

if (!checkConfig) {
  printUsage();
  process.exit(0);
}

const tauriConfigPath = join(repoRoot, "apps/desktop/src-tauri/tauri.conf.json");
const packagePath = join(repoRoot, "apps/desktop/package.json");
const tauriConfig = JSON.parse(readFileSync(tauriConfigPath, "utf8"));
const packageJson = JSON.parse(readFileSync(packagePath, "utf8"));
const failures = [];
const notes = [];

function requireCheck(condition, label, detail) {
  if (condition) {
    notes.push(`ok ${label}: ${detail}`);
  } else {
    failures.push(`${label}: ${detail}`);
  }
}

const bundle = tauriConfig.bundle ?? {};
const targets = Array.isArray(bundle.targets) ? bundle.targets : [bundle.targets];
requireCheck(bundle.active === true, "bundle.active", "Tauri bundling is enabled");
for (const target of ["app", "dmg", "msi", "nsis"]) {
  requireCheck(targets.includes(target), `bundle.targets.${target}`, `${target} target configured`);
}

requireCheck(bundle.category === "SocialNetworking", "bundle.category", "Matrix client category set");
requireCheck(Boolean(bundle.shortDescription), "bundle.shortDescription", "short bundle description set");
requireCheck(Boolean(bundle.longDescription), "bundle.longDescription", "long bundle description set");

const macOS = bundle.macOS ?? {};
requireCheck(macOS.hardenedRuntime === true, "macOS.hardenedRuntime", "hardened runtime enabled");
requireCheck(Boolean(macOS.minimumSystemVersion), "macOS.minimumSystemVersion", "minimum macOS version set");
requireCheck("signingIdentity" in macOS, "macOS.signingIdentity", "signing identity key is explicit");
requireCheck(Boolean(macOS.entitlements), "macOS.entitlements", "entitlements file configured");

const windows = bundle.windows ?? {};
requireCheck(windows.digestAlgorithm === "sha256", "windows.digestAlgorithm", "SHA-256 signing digest configured");
requireCheck(Boolean(windows.timestampUrl), "windows.timestampUrl", "timestamp server configured");
requireCheck("signCommand" in windows, "windows.signCommand", "Windows signing hook is explicit");
requireCheck(windows.allowDowngrades === false, "windows.allowDowngrades", "downgrade install blocked");
requireCheck(Boolean(windows.wix), "windows.wix", "MSI/WiX configuration present");
requireCheck(Boolean(windows.wix?.upgradeCode), "windows.wix.upgradeCode", "stable MSI upgrade code fixed");
requireCheck(Boolean(windows.nsis), "windows.nsis", "NSIS configuration present");

const security = tauriConfig.app?.security ?? {};
const assetProtocol = security.assetProtocol ?? {};
const assetProtocolScope = Array.isArray(assetProtocol.scope)
  ? assetProtocol.scope
  : Array.isArray(assetProtocol.scope?.allow)
    ? assetProtocol.scope.allow
    : [];
requireCheck(
  assetProtocol.enable === true,
  "security.assetProtocol.enable",
  "Tauri asset protocol enabled for local media/avatar images"
);
requireCheck(
  assetProtocolScope.includes("$APPDATA/**") || assetProtocolScope.includes("$APPDATA/*"),
  "security.assetProtocol.scope.appdata",
  "app data directory allowed for local media/avatar images"
);
requireCheck(
  assetProtocolScope.includes("$APPLOCALDATA/**") ||
    assetProtocolScope.includes("$APPLOCALDATA/*"),
  "security.assetProtocol.scope.applocaldata",
  "app local data directory allowed for local media/avatar images"
);
requireCheck(
  assetProtocolScope.includes("$LOCALDATA/koushi-desktop/**") ||
    assetProtocolScope.includes("$LOCALDATA/koushi-desktop/*"),
  "security.assetProtocol.scope.koushiData",
  "Koushi runtime data directory allowed for local media/avatar images"
);
for (const [label, csp] of [
  ["security.csp", security.csp],
  ["security.devCsp", security.devCsp]
]) {
  requireCheck(
    typeof csp === "string" && csp.includes("img-src") && csp.includes("asset:"),
    `${label}.img-src.asset`,
    "Tauri asset protocol allowed for local media/avatar images"
  );
  requireCheck(
    typeof csp === "string" && csp.includes("http://asset.localhost"),
    `${label}.img-src.assetLocalhost`,
    "Tauri asset localhost allowed for WebView local media/avatar images"
  );
}

requireCheck(
  packageJson.scripts?.["release:preflight"]?.includes("desktop-release-preflight"),
  "package.scripts.release:preflight",
  "npm release preflight entry exists"
);
requireCheck(
  packageJson.scripts?.["qa:manual"]?.includes("desktop-manual-qa"),
  "package.scripts.qa:manual",
  "npm manual QA entry exists"
);
requireCheck(
  packageJson.scripts?.["qa:mac-gui"]?.includes("desktop-mac-gui-smoke"),
  "package.scripts.qa:mac-gui",
  "npm macOS GUI smoke entry exists"
);
requireCheck(
  packageJson.scripts?.["qa:linux-gui"]?.includes("desktop-linux-gui-qa"),
  "package.scripts.qa:linux-gui",
  "npm Linux GUI smoke entry exists"
);
requireCheck(
  packageJson.scripts?.["qa:real-account"]?.includes("password-login-smoke") &&
    packageJson.scripts?.["qa:real-account"]?.includes("--real-account-qa"),
  "package.scripts.qa:real-account",
  "npm real-account QA smoke entry exists"
);
requireCheck(
  packageJson.scripts?.["qa:headless-local"]?.includes("desktop-headless-local-qa"),
  "package.scripts.qa:headless-local",
  "npm headless local QA entry exists"
);
requireCheck(
  packageJson.scripts?.["qa:real-homeserver"]?.includes("desktop-real-homeserver-qa"),
  "package.scripts.qa:real-homeserver",
  "npm real homeserver QA entry exists (run manually: npm --prefix apps/desktop run qa:real-homeserver)"
);

if (strictSigning) {
  requireCheck(
    Boolean(process.env.APPLE_SIGNING_IDENTITY),
    "env.APPLE_SIGNING_IDENTITY",
    "required for signed macOS distribution"
  );
  requireCheck(
    Boolean(process.env.APPLE_ID && process.env.APPLE_PASSWORD && process.env.APPLE_TEAM_ID),
    "env.appleNotarization",
    "APPLE_ID, APPLE_PASSWORD, and APPLE_TEAM_ID required for notarization"
  );
  requireCheck(
    Boolean(process.env.WINDOWS_CERTIFICATE_THUMBPRINT || process.env.WINDOWS_SIGN_COMMAND),
    "env.windowsSigning",
    "WINDOWS_CERTIFICATE_THUMBPRINT or WINDOWS_SIGN_COMMAND required for signed Windows distribution"
  );
}

for (const note of notes) {
  console.log(note);
}

if (failures.length) {
  console.error("\nrelease preflight failed");
  for (const failure of failures) {
    console.error(`missing ${failure}`);
  }
  process.exit(1);
}

console.log("release preflight passed");

function printUsage() {
  console.log("Usage: node scripts/desktop-release-preflight.mjs --check-config [--strict-signing]");
}
