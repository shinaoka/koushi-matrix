import type { CoreEventPayload, StateUpdateEnvelope } from "../domain/coreEvents";
import type { DesktopUpdateState } from "../domain/types";

export type DesktopEventUnlisten = () => void;

export interface DesktopEventPort {
  listenCoreEvents(
    listener: (payload: CoreEventPayload) => void
  ): Promise<DesktopEventUnlisten>;
  listenMenuActions(listener: (payload: string) => void): Promise<DesktopEventUnlisten>;
  listenStateUpdates(
    listener: (payload: StateUpdateEnvelope) => void
  ): Promise<DesktopEventUnlisten>;
  listenDesktopUpdates(
    listener: (payload: DesktopUpdateState) => void
  ): Promise<DesktopEventUnlisten>;
}
