import { shortcutIdForKeyboardEvent } from "../domain/shortcuts";

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

export function listenForAppShortcuts(handleShortcut: (id: string) => boolean): () => void {
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

  function onWheel(event: WheelEvent) {
    if (event.defaultPrevented || !event.ctrlKey || event.deltaY === 0) return;
    if (handleShortcut(event.deltaY < 0 ? "zoomIn" : "zoomOut")) event.preventDefault();
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
