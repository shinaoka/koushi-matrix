export interface OrderedEventBatcher<Value> {
  enqueue(value: Value): void;
  dispose(): void;
}

/**
 * Preserve transport order while collapsing a synchronous event burst into
 * one store publication. Tauri may deliver one native burst through several
 * microtasks, so the flush uses the next event-loop turn rather than another
 * microtask. This prevents React from recursively processing one update per
 * native callback without adding a visible delay to live messages.
 */
export function createOrderedEventBatcher<Value>(
  flush: (values: readonly Value[]) => void
): OrderedEventBatcher<Value> {
  let queued: Value[] = [];
  let scheduled = false;
  let disposed = false;
  let timeoutId: ReturnType<typeof setTimeout> | null = null;

  const flushQueued = () => {
    timeoutId = null;
    scheduled = false;
    if (disposed || queued.length === 0) {
      queued = [];
      return;
    }
    const values = queued;
    queued = [];
    flush(values);
  };

  return {
    enqueue(value) {
      if (disposed) return;
      queued.push(value);
      if (!scheduled) {
        scheduled = true;
        timeoutId = setTimeout(flushQueued, 0);
      }
    },
    dispose() {
      disposed = true;
      queued = [];
      if (timeoutId !== null) {
        clearTimeout(timeoutId);
        timeoutId = null;
      }
    }
  };
}
