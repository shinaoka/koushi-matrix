import type { ComposerTarget } from "./types";

export interface ComposerDraftRevisionCoordinator {
  observe(target: ComposerTarget, revision: number): void;
  nextDraft(target: ComposerTarget, authoritativeRevision: number): number;
  accept(target: ComposerTarget, submittedRevision: number): number;
  canApply(target: ComposerTarget, revision: number): boolean;
  current(target: ComposerTarget): number;
  reset(): void;
}

function targetKey(target: ComposerTarget): string {
  return target.kind === "main"
    ? `main\u0000${target.room_id}`
    : `thread\u0000${target.room_id}\u0000${target.root_event_id}`;
}

export function createComposerDraftRevisionCoordinator(): ComposerDraftRevisionCoordinator {
  const revisions = new Map<string, number>();

  function observe(target: ComposerTarget, revision: number): void {
    const key = targetKey(target);
    revisions.set(key, Math.max(revisions.get(key) ?? 0, revision));
  }

  function current(target: ComposerTarget): number {
    return revisions.get(targetKey(target)) ?? 0;
  }

  function nextDraft(target: ComposerTarget, authoritativeRevision: number): number {
    observe(target, authoritativeRevision);
    const revision = current(target) + 1;
    revisions.set(targetKey(target), revision);
    return revision;
  }

  function accept(target: ComposerTarget, submittedRevision: number): number {
    observe(target, submittedRevision);
    const revision = current(target) + 1;
    revisions.set(targetKey(target), revision);
    return revision;
  }

  return {
    observe,
    nextDraft,
    accept,
    canApply: (target, revision) => revision >= current(target),
    current,
    reset: () => revisions.clear()
  };
}
