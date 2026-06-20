declare global {
  interface Window {
    __matrixDesktopQaErrorCaptureInstalled?: boolean;
    __matrixDesktopQaLastError?: string;
  }
}

const QA_BOOTING_TITLE = "koushi-desktop qa session=booting";

if (qaTitleEnabled() && !window.__matrixDesktopQaErrorCaptureInstalled) {
  window.__matrixDesktopQaErrorCaptureInstalled = true;
  document.title = QA_BOOTING_TITLE;

  window.addEventListener("error", (event) => {
    recordBootError("error", event.error ?? event.message);
  });
  window.addEventListener("unhandledrejection", (event) => {
    recordBootError("unhandledrejection", event.reason);
  });
}

function qaTitleEnabled(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof document !== "undefined" &&
    import.meta.env.VITE_KOUSHI_QA_TITLE === "1"
  );
}

function recordBootError(source: string, reason: unknown): void {
  const errorKind = safeErrorKind(reason);
  const diagnostic = `source=${source} error_kind=${errorKind}`;
  window.__matrixDesktopQaLastError = diagnostic;
  document.title = `koushi-desktop qa session=boot_error ${diagnostic}`;
  renderBootErrorFallback(errorKind);
}

function safeErrorKind(reason: unknown): string {
  const raw =
    reason instanceof Error && reason.name.trim()
      ? reason.name
      : typeof reason === "string"
        ? "String"
        : typeof reason;
  const safe = raw.replace(/[^A-Za-z0-9_.-]/g, "_").slice(0, 48);
  return safe || "unknown";
}

function renderBootErrorFallback(errorKind: string): void {
  const root = document.getElementById("root");
  if (!root || root.childElementCount > 0) {
    return;
  }

  const fallback = document.createElement("pre");
  fallback.dataset.testid = "boot-error";
  fallback.textContent = `Koushi failed to boot (${errorKind})`;
  fallback.style.margin = "24px";
  fallback.style.padding = "16px";
  fallback.style.color = "#0f172a";
  fallback.style.background = "#fee2e2";
  fallback.style.border = "1px solid #fca5a5";
  fallback.style.borderRadius = "8px";
  fallback.style.whiteSpace = "pre-wrap";
  root.append(fallback);
}

export {};
