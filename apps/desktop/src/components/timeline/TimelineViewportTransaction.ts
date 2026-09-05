import type { ScrollAnchor } from "./TimelineViewportAnchors";

export const VIEWPORT_ANCHOR_TOLERANCE_PX = 0.5;

export type TimelineViewportTransactionPhase = "waiting-prepend" | "waiting-measurement" | "settling";
export type TimelineViewportTransactionCloseReason =
  | "input" | "key" | "generation" | "replacement" | "jump" | "live-edge" | "missing-anchor" | "unmount";
type Capture = {
  key: string;
  generation: number;
  anchor: ScrollAnchor | null;
  scrollTop?: number;
  phase?: TimelineViewportTransactionPhase;
  layoutRevision?: number;
};
export type TimelineViewportTransaction = {
  id: number;
  key: string;
  generation: number;
  inputRevision: number;
  writeGeneration: number;
  anchor: ScrollAnchor | null;
  scrollTop: number;
  phase: TimelineViewportTransactionPhase;
  layoutRevision: number;
  projectionCommitted: boolean;
  estimateWritten: boolean;
  rangePrepared: boolean;
};
type Position = { scrollTop: number; scrollHeight: number };
type WriteScope = Position & { key: string; generation: number; transactionId: number | null };
type Evidence = WriteScope & { inputRevision: number; writeGeneration: number; writing: boolean };
type StableViewport = { key: string; generation: number; anchor: ScrollAnchor; scrollTop: number };
type Fence = Pick<TimelineViewportTransaction, "id" | "key" | "generation" | "inputRevision" | "writeGeneration" | "layoutRevision">;

/** One DOM-free, renderer-lifetime owner; geometry identities never enter diagnostics. */
export function createTimelineViewportTransactionController(onLifecycle?: (message: string) => void) {
  let nextId = 0;
  let inputRevision = 0;
  let writeGeneration = 0;
  let current: TimelineViewportTransaction | null = null;
  let evidence: Evidence | null = null;
  let stable: StableViewport | null = null;
  const emit = (stage: string, reason = "none") => {
    if (current) onLifecycle?.(`stage=${stage} ordinal=${current.id} phase=${current.phase} reason=${reason}`);
  };
  const close = (reason: TimelineViewportTransactionCloseReason | "settled") => {
    emit("terminal", reason);
    current = null;
  };
  const begin = (input: Capture): TimelineViewportTransaction => {
    if (current) close("replacement");
    if (stable && (input.anchor === null || stable.key !== input.key || stable.generation !== input.generation)) stable = null;
    if (evidence && (evidence.key !== input.key || evidence.generation !== input.generation)) {
      evidence = null;
      writeGeneration += 1;
    }
    current = {
      ...input, id: ++nextId, inputRevision, writeGeneration, scrollTop: input.scrollTop ?? 0,
      phase: input.phase ?? "waiting-prepend", layoutRevision: input.layoutRevision ?? 0,
      projectionCommitted: false, estimateWritten: false, rangePrepared: false
    };
    emit("begin");
    return current;
  };
  const accountForInput = (scrollTop: number) => {
    if (stable) stable = { ...stable, anchor: { ...stable.anchor, offsetTop: stable.anchor.offsetTop - (scrollTop - stable.scrollTop) }, scrollTop };
    if (!current) return;
    if (current.anchor) {
      current.anchor = { ...current.anchor, offsetTop: current.anchor.offsetTop - (scrollTop - current.scrollTop) };
    }
    current.scrollTop = scrollTop;
  };
  return {
    accountForInput,
    stableAnchor: () => stable?.anchor ?? null,
    rememberStableAnchor(input: { key: string; generation: number; anchor: ScrollAnchor | null; scrollTop: number }) {
      stable = input.anchor ? { ...input, anchor: input.anchor } : null;
    },
    currentInputRevision: () => inputRevision,
    currentWriteGeneration: () => writeGeneration,
    active: () => current,
    begin,
    join(input: Capture) {
      if (current && current.key === input.key && current.generation === input.generation && !current.projectionCommitted) {
        current.anchor ??= input.anchor;
        if (input.phase) current.phase = input.phase;
        if (input.layoutRevision !== undefined) current.layoutRevision = input.layoutRevision;
        return current;
      }
      return begin(input);
    },
    rebase(id: number, anchor: ScrollAnchor | null) {
      if (!current || current.id !== id) return;
      current.anchor = anchor;
      current.inputRevision = inputRevision;
      current.writeGeneration = writeGeneration;
      emit("rebase", "input");
    },
    markProjectionCommitted(id: number, layoutRevision: number) {
      if (!current || current.id !== id) return false;
      current.projectionCommitted = true;
      if (current.phase !== "waiting-measurement" || layoutRevision >= current.layoutRevision) {
        current.phase = "settling";
        current.layoutRevision = layoutRevision;
      }
      return true;
    },
    markMeasurementPending(id: number, layoutRevision: number) {
      if (!current || current.id !== id) return false;
      current.phase = "waiting-measurement";
      current.layoutRevision = layoutRevision;
      emit("measurement");
      return true;
    },
    markSettling(id: number) {
      if (!current || current.id !== id) return false;
      current.phase = "settling";
      emit("settling");
      return true;
    },
    markEstimateWritten(id: number) {
      if (!current || current.id !== id || current.estimateWritten) return false;
      current.estimateWritten = true;
      emit("estimate");
      return true;
    },
    markRangePrepared(id: number) {
      if (!current || current.id !== id) return;
      current.rangePrepared = true;
    },
    markSettled(id: number) {
      if (!current || current.id !== id) return false;
      close("settled");
      return true;
    },
    userInput(scrollTop = current?.scrollTop ?? 0) {
      accountForInput(scrollTop);
      inputRevision += 1;
      writeGeneration += 1;
      evidence = null;
      if (current?.projectionCommitted && current.anchor === null) close("input");
      if (current) {
        if (!current.projectionCommitted) current.anchor = null;
        current.inputRevision = inputRevision;
        current.writeGeneration = writeGeneration;
        emit("rebase", "input");
      }
      return inputRevision;
    },
    invalidate(reason: TimelineViewportTransactionCloseReason) {
      close(reason);
      stable = null;
      evidence = null;
      writeGeneration += 1;
    },
    canWrite(fence: Fence) {
      return current !== null && current.id === fence.id && current.key === fence.key &&
        current.generation === fence.generation && current.inputRevision === inputRevision &&
        inputRevision === fence.inputRevision && current.writeGeneration === writeGeneration &&
        writeGeneration === fence.writeGeneration && current.layoutRevision === fence.layoutRevision;
    },
    write(scope: WriteScope, action: () => Position): boolean {
      if (stable && (scope.transactionId === null || stable.key !== scope.key || stable.generation !== scope.generation)) stable = null;
      const previous = evidence;
      const token = ++writeGeneration;
      if (current && current.id !== scope.transactionId) close("replacement");
      if (current) current.writeGeneration = token;
      evidence = { ...scope, inputRevision, writeGeneration: token, writing: true };
      try {
        const result = action();
        const changed = result.scrollTop !== scope.scrollTop;
        if (current?.id === scope.transactionId) current.scrollTop = result.scrollTop;
        if (stable) stable.scrollTop = result.scrollTop;
        if (evidence?.writeGeneration === token && token === writeGeneration) {
          evidence = changed ? { ...evidence, ...result, writing: false } :
            previous && previous.key === scope.key && previous.generation === scope.generation && previous.inputRevision === inputRevision
              ? { ...previous, writeGeneration: token } : null;
        }
        return changed;
      } catch (error) {
        if (evidence?.writeGeneration === token) evidence = null;
        throw error;
      }
    },
    isProgrammaticEcho(input: Position & { key: string; generation: number; hasPendingInput?: boolean }): boolean {
      if (!evidence || evidence.key !== input.key || evidence.generation !== input.generation ||
          evidence.inputRevision !== inputRevision || evidence.writeGeneration !== writeGeneration) return false;
      // Native user input cannot interleave a synchronous owned write.
      if (evidence.writing) return true;
      if (input.hasPendingInput) return false;
      if (Math.abs(evidence.scrollTop - input.scrollTop) > VIEWPORT_ANCHOR_TOLERANCE_PX) return false;
      evidence = null;
      return true;
    }
  };
}
