import type { DesktopEventPort } from "./desktopEventPort";
import { isTauriRuntime } from "./runtimeEnvironment";
import { createTauriDesktopEventPort } from "./tauri/desktopEventPort";

const tauriEventPort = createTauriDesktopEventPort();
const noEvents = async () => () => undefined;

export const desktopEventPort: DesktopEventPort = {
  listenCoreEvents: (listener) =>
    isTauriRuntime() ? tauriEventPort.listenCoreEvents(listener) : noEvents(),
  listenMenuActions: (listener) =>
    isTauriRuntime() ? tauriEventPort.listenMenuActions(listener) : noEvents(),
  listenStateUpdates: (listener) =>
    isTauriRuntime() ? tauriEventPort.listenStateUpdates(listener) : noEvents(),
  listenDesktopUpdates: (listener) =>
    isTauriRuntime() ? tauriEventPort.listenDesktopUpdates(listener) : noEvents()
};
