// jsdom has no browser top layer. This only models open/closed state for DOM tests;
// focus containment, Escape and viewport bounds are verified in headless Chromium.
if (typeof HTMLDialogElement !== "undefined") {
  HTMLDialogElement.prototype.showModal ??= function () { this.setAttribute("open", ""); };
  HTMLDialogElement.prototype.close ??= function () { this.removeAttribute("open"); };
}

// Node >= 25 predefines `localStorage` / `sessionStorage` on the global object
// (`localStorage` is a getter that yields undefined without --localstorage-file).
// Vitest's jsdom environment does not override window keys that already exist
// on the Node global, so jsdom's Web Storage never reaches the test global and
// `localStorage.clear()` throws. Install the storage objects from the real
// jsdom window; on Node 22 the globals already are these objects.
type StorageKey = "localStorage" | "sessionStorage";
const jsdomWindow = (globalThis as { jsdom?: { window: Record<StorageKey, Storage> } }).jsdom
  ?.window;
if (jsdomWindow) {
  for (const key of ["localStorage", "sessionStorage"] as const) {
    const storage = jsdomWindow[key];
    if (storage && (globalThis as Record<StorageKey, Storage | undefined>)[key] !== storage) {
      Object.defineProperty(globalThis, key, { get: () => storage, configurable: true });
    }
  }
}
