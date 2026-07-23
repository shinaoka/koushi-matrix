import { describe, expect, it } from "vitest";

import { createComposerDraftRevisionCoordinator } from "./composerDraftRevision";

const main = { kind: "main" as const, room_id: "room-a" };
const threadA = {
  kind: "thread" as const,
  room_id: "room-a",
  root_event_id: "root-a"
};
const threadB = {
  kind: "thread" as const,
  room_id: "room-a",
  root_event_id: "root-b"
};

describe("composer draft causal revisions", () => {
  it("rejects a deferred pre-send completion after accepted clear", () => {
    const revisions = createComposerDraftRevisionCoordinator();
    const sentRevision = revisions.nextDraft(main, 0);
    const acceptedRevision = revisions.accept(main, sentRevision);

    expect(acceptedRevision).toBe(2);
    expect(revisions.canApply(main, sentRevision)).toBe(false);
    expect(revisions.canApply(main, acceptedRevision)).toBe(true);
  });

  it("keeps immediate next input newer than both send and persist completions", () => {
    const revisions = createComposerDraftRevisionCoordinator();
    const sentRevision = revisions.nextDraft(main, 0);
    const acceptedRevision = revisions.accept(main, sentRevision);
    const nextRevision = revisions.nextDraft(main, acceptedRevision);

    expect(nextRevision).toBe(3);
    expect(revisions.canApply(main, sentRevision)).toBe(false);
    expect(revisions.canApply(main, acceptedRevision)).toBe(false);
    expect(revisions.canApply(main, nextRevision)).toBe(true);
  });

  it("isolates main and each thread root", () => {
    const revisions = createComposerDraftRevisionCoordinator();

    expect(revisions.nextDraft(main, 4)).toBe(5);
    expect(revisions.nextDraft(threadA, 10)).toBe(11);
    expect(revisions.nextDraft(threadB, 2)).toBe(3);
    expect(revisions.current(main)).toBe(5);
    expect(revisions.current(threadA)).toBe(11);
    expect(revisions.current(threadB)).toBe(3);
  });

  it("observes restart revisions and never moves a target backwards", () => {
    const revisions = createComposerDraftRevisionCoordinator();

    revisions.observe(main, 8);
    revisions.observe(main, 3);

    expect(revisions.current(main)).toBe(8);
    expect(revisions.nextDraft(main, 0)).toBe(9);
  });
});
