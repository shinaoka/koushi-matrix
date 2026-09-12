import { listen } from "@tauri-apps/api/event";

import type { CoreEventPayload, StateUpdateEnvelope } from "../../domain/coreEvents";
import type { DesktopUpdateState } from "../../domain/types";
import type { DesktopEventPort } from "../desktopEventPort";

const CORE_EVENT_NAME = "koushi-desktop://event";
const MENU_EVENT_NAME = "koushi-desktop://menu";
const STATE_UPDATE_EVENT_NAME = "koushi-desktop://state-update";
const DESKTOP_UPDATE_EVENT_NAME = "koushi-desktop://update";

export function createTauriDesktopEventPort(): DesktopEventPort {
  return {
    listenCoreEvents(listener) {
      return listen<CoreEventPayload>(CORE_EVENT_NAME, (event) => listener(event.payload));
    },
    listenMenuActions(listener) {
      return listen<string>(MENU_EVENT_NAME, (event) => listener(event.payload));
    },
    listenStateUpdates(listener) {
      return listen<StateUpdateEnvelope>(STATE_UPDATE_EVENT_NAME, (event) =>
        listener(event.payload)
      );
    },
    listenDesktopUpdates(listener) {
      return listen<DesktopUpdateState>(DESKTOP_UPDATE_EVENT_NAME, (event) =>
        listener(event.payload)
      );
    }
  };
}
