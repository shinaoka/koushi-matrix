/**
 * Always-on capture of uncaught JS errors and unhandled promise rejections,
 * surfaced as coarse, closed kinds in the diagnostic report. Runtime messages,
 * locations, filenames, and stack frames never enter the diagnostics buffer.
 *
 * Distinct from bootErrorCapture.ts, which is QA-window-title gated and records
 * only the error *kind* for boot detection.
 */
export type CapturedJsErrorKind =
  | "aggregate_error"
  | "error"
  | "eval_error"
  | "range_error"
  | "reference_error"
  | "syntax_error"
  | "type_error"
  | "uri_error"
  | "unknown";

export type CapturedJsErrorChannel = "window_error" | "unhandled_rejection";

export interface CapturedJsError {
  kind: CapturedJsErrorKind;
  channel: CapturedJsErrorChannel;
}

const LIMIT = 20;
let errors: CapturedJsError[] = [];

export function recordJsError(reason: unknown, channel: CapturedJsErrorChannel): void {
  errors.push({
    kind: errorKind(reason),
    channel
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
    recordJsError(event.error ?? event.message, "window_error");
  };
  const onRejection = (event: PromiseRejectionEvent) => {
    recordJsError(event.reason, "unhandled_rejection");
  };
  target.addEventListener("error", onError);
  target.addEventListener("unhandledrejection", onRejection);
  return () => {
    target.removeEventListener("error", onError);
    target.removeEventListener("unhandledrejection", onRejection);
  };
}

function errorKind(reason: unknown): CapturedJsErrorKind {
  if (reason instanceof AggregateError) return "aggregate_error";
  if (reason instanceof EvalError) return "eval_error";
  if (reason instanceof RangeError) return "range_error";
  if (reason instanceof ReferenceError) return "reference_error";
  if (reason instanceof SyntaxError) return "syntax_error";
  if (reason instanceof TypeError) return "type_error";
  if (reason instanceof URIError) return "uri_error";
  if (reason instanceof Error) return "error";
  return "unknown";
}
