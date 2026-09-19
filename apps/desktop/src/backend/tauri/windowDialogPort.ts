import { getCurrentWindow } from "@tauri-apps/api/window";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import {
  confirm as confirmDialog,
  open as openDialog,
  save as saveDialog
} from "@tauri-apps/plugin-dialog";

import type { WindowDialogPort } from "../windowDialogPort";

export function createTauriWindowDialogPort(): WindowDialogPort {
  // Transient webview presentation, in 20% steps. Commit only successful native
  // changes and serialize rapid keypresses so each uses the preceding scale.
  let zoomStep = 5;
  let pendingZoom = Promise.resolve();
  return {
    changeZoom(direction) {
      const change = pendingZoom.then(async () => {
        const next = direction === "reset"
          ? 5
          : Math.min(50, Math.max(1, zoomStep + (direction === "in" ? 1 : -1)));
        await getCurrentWebview().setZoom(next / 5);
        zoomStep = next;
        // Native window buttons do not zoom with the WebView. Layout reads the
        // successful scale to reserve enough CSS space below those controls.
        document.documentElement.style.setProperty("--webview-zoom", String(next / 5));
        window.dispatchEvent(new Event("resize"));
      });
      pendingZoom = change.catch(() => {});
      return change;
    },
    async toggleFullscreen() {
      const window = getCurrentWindow();
      const fullscreen = await window.isFullscreen();
      await window.setFullscreen(!fullscreen);
    },
    startDragging() {
      return getCurrentWindow().startDragging();
    },
    confirm(message, options) {
      return confirmDialog(message, options);
    },
    saveFile(options) {
      return saveDialog(options);
    },
    openFile(options) {
      return openDialog(options);
    }
  };
}
