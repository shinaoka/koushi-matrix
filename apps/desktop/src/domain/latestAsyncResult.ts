export interface LatestAsyncResultToken {
  isCurrent(): boolean;
  settle(): void;
}

export interface LatestAsyncResultGate<Key> {
  begin(key: Key): LatestAsyncResultToken;
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
