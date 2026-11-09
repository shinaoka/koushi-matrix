import { useRef, useState } from "react";
import { api } from "../backend/appRuntime";
import { getAppStoreSnapshot, setAppStoreSnapshot, useAppStore } from "../domain/appStore";
import { createCommandReceiptReconciler } from "../domain/commandWatermark";

/** All picker entry points share the Rust-owned, device-local window preference. */
export function useEmojiPickerSize() {
  const size = useAppStore(store => store.snapshot?.state.domain.settings.values.window.emoji_picker_size);
  const failureCount = useAppStore(store => store.snapshot?.state.ui.errors.filter(error => error.code === "settings_persist_failed").length ?? 0);
  const previousFailures = useRef(failureCount);
  const [commandFailed, setCommandFailed] = useState(false);
  const reconcile = useRef(createCommandReceiptReconciler({
    currentGeneration: () => getAppStoreSnapshot()?.state_generation ?? null,
    settlementSnapshot: () => api.settlementSnapshot(),
    applySnapshot: setAppStoreSnapshot,
  }));
  const save = (next: [number, number] | null) => {
    const current = getAppStoreSnapshot()?.state.domain.settings.values.window;
    if (!current) return;
    previousFailures.current = failureCount;
    setCommandFailed(false);
    void api.updateSettings({ scope: "app", window: { ...current, emoji_picker_size: next } })
      .then(receipt => reconcile.current(receipt))
      .catch(() => setCommandFailed(true));
  };
  return { size, save, failed: commandFailed || failureCount > previousFailures.current };
}
