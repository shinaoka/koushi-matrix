#!/usr/bin/env node
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const strictSigning = process.argv.includes("--strict-signing");
const macosSigning = process.argv.includes("--macos-signing") || strictSigning;
const checkConfig = process.argv.includes("--check-config") || macosSigning;

if (!checkConfig) {
  printUsage();
  process.exit(0);
}

const tauriConfigPath = join(repoRoot, "apps/desktop/src-tauri/tauri.conf.json");
const tauriMacosConfigPath = join(repoRoot, "apps/desktop/src-tauri/tauri.macos.conf.json");
const packagePath = join(repoRoot, "apps/desktop/package.json");
const tauriConfig = JSON.parse(readFileSync(tauriConfigPath, "utf8"));
const tauriMacosConfig = JSON.parse(readFileSync(tauriMacosConfigPath, "utf8"));
const packageJson = JSON.parse(readFileSync(packagePath, "utf8"));
const minimumSecureWebView2Version = "155.0.4283.45";
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
requireCheck(
  tauriMacosConfig.bundle?.createUpdaterArtifacts === true,
  "macOS.createUpdaterArtifacts",
  "signed macOS updater archives are enabled"
);
requireCheck(
  tauriMacosConfig.plugins?.updater?.endpoints?.includes(
    "https://github.com/shinaoka/koushi-matrix/releases/latest/download/latest.json"
  ),
  "macOS.updater.endpoint",
  "stable GitHub Release manifest endpoint configured"
);

const windows = bundle.windows ?? {};
requireCheck(windows.digestAlgorithm === "sha256", "windows.digestAlgorithm", "SHA-256 signing digest configured");
requireCheck(Boolean(windows.timestampUrl), "windows.timestampUrl", "timestamp server configured");
requireCheck("signCommand" in windows, "windows.signCommand", "Windows signing hook is explicit");
requireCheck(windows.allowDowngrades === false, "windows.allowDowngrades", "downgrade install blocked");
requireCheck(
  windows.minimumWebview2Version === minimumSecureWebView2Version,
  "windows.minimumWebview2Version",
  `WebView2 security floor fixed at ${minimumSecureWebView2Version}`
);
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
  !assetProtocolScope.some(
    (scope) => scope === "$APPDATA/**" || scope === "$APPDATA/*"
  ),
  "security.assetProtocol.scope.noBroadAppdata",
  "broad app data asset scope is not allowed"
);
requireCheck(
  !assetProtocolScope.some(
    (scope) => scope === "$APPLOCALDATA/**" || scope === "$APPLOCALDATA/*"
  ),
  "security.assetProtocol.scope.noBroadAppLocalData",
  "broad app-local data asset scope is not allowed"
);
requireCheck(
  assetProtocolScope.includes("$LOCALDATA/koushi-desktop/accounts/v2/*/media-downloads/**"),
  "security.assetProtocol.scope.mediaDownloads",
  "only account-local media downloads are statically exposed through the asset protocol"
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
  requireCheck(
    typeof csp === "string" && csp.includes("koushi-thumbnail:"),
    `${label}.img-src.koushiThumbnail`,
    "custom in-memory thumbnail protocol allowed for avatar/link-preview images"
  );
  requireCheck(
    typeof csp === "string" && csp.includes("koushi-media:"),
    `${label}.img-src.koushiMedia`,
    "custom managed-download protocol allowed for timeline media images"
  );
  // #2096: the inline player loads the same managed download through a media
  // element, which `default-src 'none'` would otherwise block. Both the scheme
  // and the Windows `http://<protocol>.localhost` form are required: a CSP that
  // lists only one of them leaves the player dead on the other platform.
  const mediaSrc = /(?:^|;\s*)media-src ([^;]*)/.exec(typeof csp === "string" ? csp : "")?.[1] ?? "";
  requireCheck(
    mediaSrc.includes("koushi-media:"),
    `${label}.media-src`,
    "custom managed-download protocol allowed for inline audio and video"
  );
  requireCheck(
    mediaSrc.includes("http://koushi-media.localhost"),
    `${label}.media-src.windowsOrigin`,
    "Windows serves the managed-download protocol as http://koushi-media.localhost"
  );
  // #2096: the PDF viewer's pdf.js worker is created from a blob: URL, which
  // `default-src 'none'` blocks. `script-src` must stay as it is; only the
  // worker directive is widened, and only for same-origin and blob: workers.
  const workerSrc = /(?:^|;\s*)worker-src ([^;]*)/.exec(typeof csp === "string" ? csp : "")?.[1] ?? "";
  requireCheck(
    workerSrc.split(/\s+/).includes("'self'") && workerSrc.split(/\s+/).includes("blob:"),
    `${label}.worker-src`,
    "same-origin and blob: workers allowed for the PDF viewer"
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
  packageJson.scripts?.["qa:headless-local"]?.includes("desktop-headless-local-qa"),
  "package.scripts.qa:headless-local",
  "npm headless local QA entry exists"
);

if (macosSigning) {
  requireCheck(
    Boolean(process.env.KOUSHI_UPDATER_PUBLIC_KEY),
    "env.KOUSHI_UPDATER_PUBLIC_KEY",
    "required to verify downloaded macOS updates"
  );
  requireCheck(
    Boolean(process.env.TAURI_SIGNING_PRIVATE_KEY),
    "env.TAURI_SIGNING_PRIVATE_KEY",
    "required to sign macOS updater archives"
  );
  requireCheck(
    Boolean(process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD),
    "env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD",
    "required to unlock the macOS updater signing key"
  );
  requireCheck(
    Boolean(process.env.APPLE_SIGNING_IDENTITY),
    "env.APPLE_SIGNING_IDENTITY",
    "required for signed macOS distribution"
  );
  const apiKeyNotarization = Boolean(
    process.env.APPLE_API_ISSUER &&
      process.env.APPLE_API_KEY &&
      process.env.APPLE_API_KEY_PATH &&
      existsSync(process.env.APPLE_API_KEY_PATH)
  );
  const appleIdNotarization = Boolean(
    process.env.APPLE_ID && process.env.APPLE_PASSWORD && process.env.APPLE_TEAM_ID
  );
  requireCheck(
    apiKeyNotarization || appleIdNotarization,
    "env.appleNotarization",
    "complete App Store Connect API key or Apple ID credential set required for notarization"
  );
}
if (strictSigning) {
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
  console.log("Usage: node scripts/desktop-release-preflight.mjs --check-config [--macos-signing | --strict-signing]");
}
