export type SubmissionId = string;

export interface ComposerSubmissionController {
  begin(): SubmissionId | null;
  accept(submissionId: SubmissionId): void;
  reject(submissionId: SubmissionId): void;
  releaseTerminal(submissionId: SubmissionId): void;
  active(): SubmissionId | null;
}

export function createSubmissionId(): SubmissionId {
  return `submission-${crypto.randomUUID()}`;
}

export function createComposerSubmissionController(
  createId: () => SubmissionId = createSubmissionId
): ComposerSubmissionController {
  let current: { id: SubmissionId; accepted: boolean } | null = null;
  return {
    begin() {
      if (current !== null) {
        return null;
      }
      const id = createId();
      current = { id, accepted: false };
      return id;
    },
    accept(submissionId) {
      if (current?.id === submissionId) {
        current.accepted = true;
      }
    },
    reject(submissionId) {
      if (current?.id === submissionId && !current.accepted) {
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
