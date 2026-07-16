export interface LatestAsyncResultToken {
  isCurrent(): boolean;
  settle(): void;
}

export interface LatestAsyncResultGate<Key> {
  begin(key: Key): LatestAsyncResultToken;
  invalidate(key: Key): void;
}

export type LatestAsyncOperationResult<Value> =
  | { kind: "applied"; value: Value }
  | { kind: "superseded" };

export interface LatestAsyncOperationQueue<Key> {
  run<Value>(
    key: Key,
    operation: () => Promise<Value>
  ): Promise<LatestAsyncOperationResult<Value>>;
  invalidate(key: Key): void;
}

export function createLatestAsyncResultGate<Key>(): LatestAsyncResultGate<Key> {
  const generations = new Map<Key, number>();
  let nextGeneration = 0;

  return {
    begin(key) {
      nextGeneration += 1;
      const generation = nextGeneration;
      generations.set(key, generation);
      return {
        isCurrent: () => generations.get(key) === generation,
        settle() {
          if (generations.get(key) === generation) {
            generations.delete(key);
          }
        }
      };
    },
    invalidate(key) {
      generations.delete(key);
    }
  };
}

export function createLatestAsyncOperationQueue<Key>(): LatestAsyncOperationQueue<Key> {
  const gate = createLatestAsyncResultGate<Key>();
  const tails = new Map<Key, Promise<void>>();

  return {
    async run<Value>(key: Key, operation: () => Promise<Value>) {
      const token = gate.begin(key);
      const previous = tails.get(key) ?? Promise.resolve();
      const task = previous.then(async (): Promise<LatestAsyncOperationResult<Value>> => {
        if (!token.isCurrent()) {
          return { kind: "superseded" };
        }
        const value = await operation();
        return token.isCurrent()
          ? { kind: "applied", value }
          : { kind: "superseded" };
      });
      const tail = task.then(
        () => undefined,
        () => undefined
      );
      tails.set(key, tail);

      try {
        return await task;
      } finally {
        token.settle();
        if (tails.get(key) === tail) {
          tails.delete(key);
        }
      }
    },
    invalidate(key) {
      gate.invalidate(key);
    }
  };
}
