/**
 * Always-on capture of uncaught JS errors and unhandled promise rejections,
 * surfaced (sanitized) in the diagnostic report. This makes runtime exceptions
 * visible from the diagnostics text alone, without attaching a Web Inspector.
 *
 * Distinct from bootErrorCapture.ts, which is QA-window-title gated and records
 * only the error *kind* for boot detection. Messages here are private-data-free:
 * room/user/event ids are stripped and length is capped.
 */
export interface CapturedJsError {
  kind: string;
  message: string;
  source: string;
}

const LIMIT = 20;
let errors: CapturedJsError[] = [];

export function recordJsError(reason: unknown, location?: string): void {
  errors.push({
    kind: errorKind(reason),
    message: sanitizeErrorText(errorMessage(reason)),
    source: sanitizeErrorText(location ?? errorSource(reason))
  });
  if (errors.length > LIMIT) {
    errors = errors.slice(-LIMIT);
  }
}

export function getRecentJsErrors(): CapturedJsError[] {
  return [...errors];
}

export function resetJsErrors(): void {
  errors = [];
}

export function installJsErrorCapture(target: Window): () => void {
  const onError = (event: ErrorEvent) => {
    recordJsError(
      event.error ?? event.message,
      formatLocation(event.filename, event.lineno, event.colno)
    );
  };
  const onRejection = (event: PromiseRejectionEvent) => {
    recordJsError(event.reason);
  };
  target.addEventListener("error", onError);
  target.addEventListener("unhandledrejection", onRejection);
  return () => {
    target.removeEventListener("error", onError);
    target.removeEventListener("unhandledrejection", onRejection);
  };
}

function errorKind(reason: unknown): string {
  const raw =
    reason instanceof Error && reason.name.trim()
      ? reason.name
      : typeof reason === "string"
        ? "String"
        : typeof reason;
  return raw.replace(/[^A-Za-z0-9_.-]/g, "_").slice(0, 48) || "unknown";
}

function errorMessage(reason: unknown): string {
  if (reason instanceof Error) {
    return reason.message;
  }
  if (typeof reason === "string") {
    return reason;
  }
  return "";
}

function errorSource(reason: unknown): string {
  if (reason instanceof Error && typeof reason.stack === "string") {
    const frame = reason.stack
      .split("\n")
      .map((line) => line.trim())
      .find((line) => line.startsWith("at ") || line.includes("@"));
    if (frame) {
      return frame;
    }
  }
  return "";
}

function formatLocation(filename: string, lineno: number, colno: number): string {
  if (!filename) {
    return "";
  }
  return `${filename}:${lineno}:${colno}`;
}

function sanitizeErrorText(value: string): string {
  return value
    .replace(/![^\s'"`)]+/g, "<room>")
    .replace(/@[^\s'"`)]+/g, "<user>")
    .replace(/\$[^\s'"`)]+/g, "<event>")
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, 200);
}
