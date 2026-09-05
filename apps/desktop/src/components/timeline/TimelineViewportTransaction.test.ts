import { describe, expect, it } from "vitest";
import { createTimelineViewportTransactionController } from "./TimelineViewportTransaction";

const capture = { key: "synthetic-key", generation: 1, anchor: { itemId: "row", offsetTop: -5 } };
const position = { key: capture.key, generation: 1, scrollTop: 100, scrollHeight: 1000 };
function write(owner: ReturnType<typeof createTimelineViewportTransactionController>, transactionId: number | null) {
  owner.write({ ...position, scrollTop: 0, transactionId }, () => position);
}

describe("viewport transaction fences", () => {
  it("retains the stable target across completion and counts only subsequent user movement", () => {
    const owner = createTimelineViewportTransactionController();
    owner.rememberStableAnchor({ ...capture, scrollTop: 100 });
    const tx = owner.begin({ ...capture, scrollTop: 100 });
    owner.write({ ...position, transactionId: tx.id }, () => ({ ...position, scrollTop: 180 }));
    owner.markSettled(tx.id);
    owner.isProgrammaticEcho({ ...position, scrollTop: 180 });
    owner.userInput(178);
    expect(owner.stableAnchor()?.offsetTop).toBe(-3);
    owner.accountForInput(176);
    owner.userInput(176);
    expect(owner.stableAnchor()?.offsetTop).toBe(-1);
    owner.invalidate("jump");
    expect(owner.stableAnchor()).toBeNull();
  });
  it("clears stable geometry at a different timeline scope", () => {
    const owner = createTimelineViewportTransactionController();
    owner.rememberStableAnchor({ ...capture, scrollTop: 100 });
    owner.begin({ ...capture, key: "other-key" });
    expect(owner.stableAnchor()).toBeNull();
  });
  it("invalidates programmatic evidence on jumps even after terminal settlement", () => {
    const owner = createTimelineViewportTransactionController();
    const tx = owner.begin(capture);
    write(owner, tx.id);
    owner.markSettled(tx.id);
    owner.invalidate("jump");
    expect(owner.isProgrammaticEcho(position)).toBe(false);
  });
  it("consumes an own echo without advancing user revision", () => {
    const owner = createTimelineViewportTransactionController();
    const tx = owner.begin(capture);
    write(owner, tx.id);
    owner.markSettled(tx.id);
    expect(owner.isProgrammaticEcho(position)).toBe(true);
    expect(owner.currentInputRevision()).toBe(0);
    expect(owner.isProgrammaticEcho(position)).toBe(false);
  });
  it("cannot restore a transaction after an unrelated write took the viewport", () => {
    const owner = createTimelineViewportTransactionController();
    const tx = owner.begin(capture);
    const captured = { ...tx };
    write(owner, null);
    expect(owner.canWrite(captured)).toBe(false);
  });
  it("fences input before scroll and rebases only uncommitted geometry", () => {
    const owner = createTimelineViewportTransactionController();
    const tx = owner.begin(capture);
    const captured = { ...tx };
    owner.userInput();
    expect(owner.canWrite(captured)).toBe(false);
    expect(owner.active()?.anchor).toBeNull();
    owner.rebase(tx.id, { itemId: "new-row", offsetTop: 3 });
    owner.markProjectionCommitted(tx.id, 2);
    expect(owner.canWrite({ ...tx })).toBe(true);
    const committedFence = { ...tx };
    owner.userInput(10);
    expect(owner.canWrite(committedFence)).toBe(false);
    expect(owner.active()?.anchor?.offsetTop).toBe(-7);
    expect(owner.canWrite({ ...tx })).toBe(true);
  });
  it("does not retain an anchorless committed transaction when input takes control", () => {
    const owner = createTimelineViewportTransactionController();
    const tx = owner.begin({ ...capture, anchor: null });
    owner.markProjectionCommitted(tx.id, 1);
    owner.userInput(100);
    expect(owner.active()).toBeNull();
  });
  it("accounts delayed input exactly once and excludes the estimated write", () => {
    const owner = createTimelineViewportTransactionController();
    const tx = owner.begin({ ...capture, scrollTop: 100 });
    owner.markProjectionCommitted(tx.id, 1);
    owner.userInput(98);
    owner.accountForInput(96); // Movement precedes the delayed native event.
    owner.userInput(96);
    expect(tx.anchor?.offsetTop).toBe(-1);
    owner.write({ ...position, scrollTop: 96, transactionId: tx.id }, () => ({ ...position, scrollTop: 596 }));
    owner.userInput(594);
    expect(tx.anchor?.offsetTop).toBe(1);
    expect(tx.scrollTop).toBe(594);
  });
  it("bounds estimate and finalization to the same current transaction", () => {
    const owner = createTimelineViewportTransactionController();
    const tx = owner.begin(capture);
    owner.markProjectionCommitted(tx.id, 1);
    expect(owner.markEstimateWritten(tx.id)).toBe(true);
    const beforeEstimate = { ...tx };
    write(owner, tx.id);
    expect(owner.canWrite(beforeEstimate)).toBe(false);
    expect(owner.canWrite({ ...tx })).toBe(true);
    expect(owner.markEstimateWritten(tx.id)).toBe(false);
    expect(owner.markSettled(tx.id)).toBe(true);
    expect(owner.markSettled(tx.id)).toBe(false);
    expect(owner.canWrite({ ...tx })).toBe(false);
  });
  it("joins before DOM commit but replaces later pages and generations", () => {
    const owner = createTimelineViewportTransactionController();
    const first = owner.begin(capture);
    expect(owner.join({ ...capture, anchor: null }).id).toBe(first.id);
    owner.markProjectionCommitted(first.id, 1);
    const next = owner.join(capture);
    expect(next.id).not.toBe(first.id);
    expect(owner.canWrite(first)).toBe(false);
    const reset = owner.begin({ ...capture, generation: 2 });
    expect(owner.canWrite(next)).toBe(false);
    expect(owner.canWrite({ ...reset, generation: 1 })).toBe(false);
    expect(owner.canWrite({ ...reset, key: "another-key" })).toBe(false);
  });
  it("classifies a synchronous own echo only within the writing scope", () => {
    const owner = createTimelineViewportTransactionController();
    const tx = owner.begin(capture);
    owner.write({ ...position, scrollTop: 0, transactionId: tx.id }, () => {
      expect(owner.isProgrammaticEcho({ ...position, hasPendingInput: true })).toBe(true);
      return position;
    });
    owner.userInput();
    expect(owner.isProgrammaticEcho(position)).toBe(false);
  });
  it("reports closed lifecycle tokens without retaining an identifier-bearing history", () => {
    const messages: string[] = [];
    const owner = createTimelineViewportTransactionController((message) => messages.push(message));
    const tx = owner.begin({ ...capture, key: "!private:example.invalid", anchor: { itemId: "$secret", offsetTop: -5 } });
    owner.markMeasurementPending(tx.id, 1);
    owner.markSettling(tx.id);
    owner.markEstimateWritten(tx.id);
    owner.markSettled(tx.id);
    expect(messages).toHaveLength(5);
    expect(messages.join(" ")).not.toMatch(/private|secret|offset/);
    expect(messages.at(-1)).toContain("reason=settled");
    expect(owner.active()).toBeNull();
  });
});
