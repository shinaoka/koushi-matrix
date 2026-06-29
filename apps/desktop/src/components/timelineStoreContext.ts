import {
  createContext,
  useContext,
  type Dispatch,
  type SetStateAction
} from "react";

import type { TimelineKeyState, TimelineStoreState } from "../domain/timelineStore";

export interface TimelineStoreContextValue {
  getSnapshot: () => TimelineStoreState;
  getKeyStateById: (keyId: string) => TimelineKeyState | undefined;
  setStore: Dispatch<SetStateAction<TimelineStoreState>>;
  subscribe: (listener: () => void) => () => void;
}

export const TimelineStoreContext = createContext<TimelineStoreContextValue | null>(null);

export function useTimelineStoreContext(): TimelineStoreContextValue | null {
  return useContext(TimelineStoreContext);
}

export function createTimelineStoreController(
  initialStore: TimelineStoreState
): TimelineStoreContextValue {
  let store = initialStore;
  const listeners = new Set<() => void>();

  const notify = () => {
    for (const listener of listeners) {
      listener();
    }
  };

  return {
    getSnapshot: () => store,
    getKeyStateById: (keyId) => store.keys.get(keyId),
    setStore(update) {
      const next =
        typeof update === "function"
          ? (update as (current: TimelineStoreState) => TimelineStoreState)(store)
          : update;
      if (next === store) {
        return;
      }
      store = next;
      notify();
    },
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    }
  };
}
