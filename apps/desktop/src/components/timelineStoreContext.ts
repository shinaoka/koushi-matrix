import {
  createContext,
  useContext,
  type Dispatch,
  type SetStateAction
} from "react";

import type { TimelineStoreState } from "../domain/timelineStore";

export interface TimelineStoreContextValue {
  store: TimelineStoreState;
  setStore: Dispatch<SetStateAction<TimelineStoreState>>;
}

export const TimelineStoreContext = createContext<TimelineStoreContextValue | null>(null);

export function useTimelineStoreContext(): TimelineStoreContextValue | null {
  return useContext(TimelineStoreContext);
}
