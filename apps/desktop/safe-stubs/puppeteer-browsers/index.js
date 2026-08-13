const disabled = () => {
  throw new Error(
    "Automatic browser download is disabled in Koushi; desktop QA connects to the prestarted tauri-driver"
  );
};

export const Browser = Object.freeze({
  CHROME: "chrome",
  CHROMEHEADLESSSHELL: "chrome-headless-shell",
  CHROMIUM: "chromium",
  FIREFOX: "firefox"
});
export const ChromeReleaseChannel = Object.freeze({
  STABLE: "stable",
  BETA: "beta",
  DEV: "dev",
  CANARY: "canary"
});
export const install = disabled;
export const canDownload = disabled;
export const resolveBuildId = disabled;
export const detectBrowserPlatform = disabled;
export const computeExecutablePath = disabled;
