export type SubmissionId = string;

export interface ComposerSubmissionController {
  begin(): SubmissionId | null;
  capture<T>(submissionId: SubmissionId, payload: T): void;
  payload<T>(submissionId: SubmissionId): T | undefined;
  accept(submissionId: SubmissionId): void;
  reject(submissionId: SubmissionId): void;
  markUnknown(submissionId: SubmissionId, reason: SubmissionUnknownReason): void;
  unknownReason(submissionId: SubmissionId): SubmissionUnknownReason | null;
  observeSnapshot(
    submissionId: SubmissionId,
    acceptedSubmissionIds: readonly string[],
    pendingSubmissionId: string | null | undefined
  ): void;
  releaseTerminal(submissionId: SubmissionId): void;
  active(): SubmissionId | null;
}

export type SubmissionUnknownReason = "timeout" | "disconnected" | "lagged" | "submitFailed";
export type SubmissionFailureDisposition =
  | { kind: "unknown"; reason: SubmissionUnknownReason }
  | { kind: "rejected"; reason: string };

export function classifySubmissionFailure(error: unknown): SubmissionFailureDisposition {
  const reason = typeof error === "string" ? error : "submitFailed";
  if (reason === "timeout" || reason === "disconnected" || reason === "lagged" || reason === "submitFailed") {
    return { kind: "unknown", reason };
  }
  return { kind: "rejected", reason };
}

export function createSubmissionId(): SubmissionId {
  return `submission-${crypto.randomUUID()}`;
}

export function createComposerSubmissionController(
  createId: () => SubmissionId = createSubmissionId
): ComposerSubmissionController {
  let current: {
    id: SubmissionId;
    accepted: boolean;
    unknownReason: SubmissionUnknownReason | null;
    payload?: unknown;
  } | null = null;
  return {
    begin() {
      if (current?.unknownReason !== null && current !== null) {
        return current.id;
      }
      if (current !== null) {
        return null;
      }
      const id = createId();
      current = { id, accepted: false, unknownReason: null };
      return id;
    },
    capture(submissionId, payload) {
      if (current?.id === submissionId && current.payload === undefined) {
        current.payload = payload;
      }
    },
    payload<T>(submissionId: SubmissionId) {
      return current?.id === submissionId ? (current.payload as T | undefined) : undefined;
    },
    accept(submissionId) {
      if (current?.id === submissionId) {
        current.accepted = true;
        current.unknownReason = null;
      }
    },
    reject(submissionId) {
      if (current?.id === submissionId && !current.accepted) {
        current = null;
      }
    },
    markUnknown(submissionId, reason) {
      if (current?.id === submissionId && !current.accepted) {
        current.unknownReason = reason;
      }
    },
    unknownReason(submissionId) {
      return current?.id === submissionId ? current.unknownReason : null;
    },
    observeSnapshot(submissionId, acceptedSubmissionIds, pendingSubmissionId) {
      if (current?.id !== submissionId || !acceptedSubmissionIds.includes(submissionId)) {
        return;
      }
      current.accepted = true;
      current.unknownReason = null;
      if (pendingSubmissionId !== submissionId) {
        current = null;
      }
    },
    releaseTerminal(submissionId) {
      if (current?.id === submissionId && current.accepted) {
        current = null;
      }
    },
    active() {
      return current?.id ?? null;
    }
  };
}
