import {
  defaultShortcutLabelProfile,
  shortcutIdForKeyboardEvent,
  type ShortcutPlatformProfile
} from "../domain/shortcuts";

const windowShortcutIds = new Set([
  "toggleFullscreen",
  "zoomIn",
  "zoomOut",
  "resetZoom",
  // App Settings is an app-level destination like the window operations, so its
  // standard shortcut stays reachable while a modal settings dialog is open.
  "openAppSettings"
]);

// Global listeners are installed once, so they must not close over one render's
// handler. Re-register the handler every render and dispatch through this box.
export function createLatestShortcutHandler(): {
  register(handler: (id: string) => boolean): void;
  handle(id: string): boolean;
} {
  let current: (id: string) => boolean = () => false;
  return {
    register(handler) {
      current = handler;
    },
    handle(id) {
      return current(id);
    }
  };
}

// One mouse-wheel notch in pixel delta mode (Chromium/WebView2 and WebKitGTK).
const WHEEL_ZOOM_STEP_PIXELS = 100;

function wheelDeltaModePixels(deltaMode: number): number {
  if (deltaMode === WheelEvent.DOM_DELTA_LINE) return WHEEL_ZOOM_STEP_PIXELS;
  if (deltaMode === WheelEvent.DOM_DELTA_PAGE) return WHEEL_ZOOM_STEP_PIXELS * 10;
  return 1;
}

export function listenForAppShortcuts(
  handleShortcut: (id: string) => boolean,
  platform: ShortcutPlatformProfile = defaultShortcutLabelProfile().platform
): () => void {
  // App-level and window operations remain available in dialogs and focused
  // widgets that stop bubbling keys. Room/composer shortcuts still defer to
  // those local handlers.
  function onWindowKeyDown(event: KeyboardEvent) {
    if (event.defaultPrevented) return;
    const shortcutId = shortcutIdForKeyboardEvent(event);
    if (shortcutId && windowShortcutIds.has(shortcutId) && handleShortcut(shortcutId)) {
      event.preventDefault();
    }
  }

  function onKeyDown(event: KeyboardEvent) {
    if (event.defaultPrevented || document.querySelector("dialog[open]")) return;
    const shortcutId = shortcutIdForKeyboardEvent(event);
    if (shortcutId && !windowShortcutIds.has(shortcutId) && handleShortcut(shortcutId)) {
      event.preventDefault();
    }
  }

  // #1694: a trackpad pinch arrives as a burst of Ctrl+wheel events with small
  // pixel deltas. On macOS that is the only common source, and Cmd+=/- is the
  // zoom gesture, so the pinch is consumed without zooming. Elsewhere Ctrl+wheel
  // zooms one step per mouse-notch of accumulated travel.
  let wheelZoomTravel = 0;
  function onWheel(event: WheelEvent) {
    if (event.defaultPrevented || !event.ctrlKey || event.deltaY === 0) return;
    if (platform === "macos") {
      event.preventDefault();
      return;
    }
    const delta = event.deltaY * wheelDeltaModePixels(event.deltaMode);
    if (Math.sign(delta) !== Math.sign(wheelZoomTravel)) wheelZoomTravel = 0;
    wheelZoomTravel += delta;
    event.preventDefault();
    if (Math.abs(wheelZoomTravel) < WHEEL_ZOOM_STEP_PIXELS) return;
    wheelZoomTravel = 0;
    handleShortcut(delta < 0 ? "zoomIn" : "zoomOut");
  }

  window.addEventListener("keydown", onWindowKeyDown, true);
  window.addEventListener("keydown", onKeyDown);
  window.addEventListener("wheel", onWheel, { capture: true, passive: false });
  return () => {
    window.removeEventListener("keydown", onWindowKeyDown, true);
    window.removeEventListener("keydown", onKeyDown);
    window.removeEventListener("wheel", onWheel, true);
  };
}
