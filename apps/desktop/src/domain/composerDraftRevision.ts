import type { ComposerDraftRevision, ComposerTarget } from "./types";

const U128_MAX = 340282366920938463463374607431768211455n;
const CANONICAL_DECIMAL = /^(0|[1-9][0-9]{0,38})$/;

export const COMPOSER_DRAFT_REVISION_ZERO = "0" as ComposerDraftRevision;

export class ComposerDraftRevisionExhaustedError extends Error {
  readonly kind = "composer_revision_exhausted";

  constructor() {
    super("composer revision exhausted");
    this.name = "ComposerDraftRevisionExhaustedError";
  }
}

export function parseComposerDraftRevision(value: unknown): ComposerDraftRevision {
  if (
    typeof value !== "string" ||
    !CANONICAL_DECIMAL.test(value) ||
    BigInt(value) > U128_MAX
  ) {
    throw new TypeError("invalid composer revision");
  }
  return value as ComposerDraftRevision;
}

export function compareComposerDraftRevisions(
  left: ComposerDraftRevision,
  right: ComposerDraftRevision
): number {
  const leftValue = BigInt(left);
  const rightValue = BigInt(right);
  return leftValue < rightValue ? -1 : leftValue > rightValue ? 1 : 0;
}

export function nextComposerDraftRevision(
  authoritative: ComposerDraftRevision,
  submitted: ComposerDraftRevision
): ComposerDraftRevision {
  const current =
    compareComposerDraftRevisions(authoritative, submitted) >= 0 ? authoritative : submitted;
  const value = BigInt(current);
  if (value === U128_MAX) {
    throw new ComposerDraftRevisionExhaustedError();
  }
  return parseComposerDraftRevision((value + 1n).toString());
}

export interface ComposerDraftRevisionCoordinator {
  observe(target: ComposerTarget, revision: ComposerDraftRevision): void;
  nextDraft(
    target: ComposerTarget,
    authoritativeRevision: ComposerDraftRevision
  ): ComposerDraftRevision;
  accept(target: ComposerTarget, submittedRevision: ComposerDraftRevision): ComposerDraftRevision;
  canApply(target: ComposerTarget, revision: ComposerDraftRevision): boolean;
  current(target: ComposerTarget): ComposerDraftRevision;
  reset(): void;
}

function targetKey(target: ComposerTarget): string {
  return target.kind === "main"
    ? `main\u0000${target.room_id}`
    : `thread\u0000${target.room_id}\u0000${target.root_event_id}`;
}

export function createComposerDraftRevisionCoordinator(): ComposerDraftRevisionCoordinator {
  const revisions = new Map<string, ComposerDraftRevision>();

  function observe(target: ComposerTarget, revision: ComposerDraftRevision): void {
    const key = targetKey(target);
    const previous = revisions.get(key) ?? COMPOSER_DRAFT_REVISION_ZERO;
    if (compareComposerDraftRevisions(revision, previous) > 0) {
      revisions.set(key, revision);
    }
  }

  function current(target: ComposerTarget): ComposerDraftRevision {
    return revisions.get(targetKey(target)) ?? COMPOSER_DRAFT_REVISION_ZERO;
  }

  function advance(
    target: ComposerTarget,
    revision: ComposerDraftRevision
  ): ComposerDraftRevision {
    const next = nextComposerDraftRevision(current(target), revision);
    revisions.set(targetKey(target), next);
    return next;
  }

  return {
    observe,
    nextDraft: advance,
    accept: advance,
    canApply: (target, revision) =>
      compareComposerDraftRevisions(revision, current(target)) >= 0,
    current,
    reset: () => revisions.clear()
  };
}
