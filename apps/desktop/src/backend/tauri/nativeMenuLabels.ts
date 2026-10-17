import { invoke } from "@tauri-apps/api/core";

import { t, type Locale, type MessageId, type PseudoLocaleMode } from "../../i18n/messages";
import { isTauriRuntime } from "../runtimeEnvironment";

// The native menu is built in Tauri setup, before this webview knows the
// catalog locale. Rust owns the menu structure and the catalog message ids;
// it asks for the resolved strings, so a locale change re-applies the menu.
let pending = Promise.resolve();

export function syncNativeMenuLabels(
  locale: Locale,
  pseudoLocale: PseudoLocaleMode | "none" = "none",
  /** #1504: the selected signed-in account, named by account-scoped items. */
  account: string | null = null
): Promise<void> {
  if (!isTauriRuntime()) return Promise.resolve();
  // Serialize rebuilds so a slow older locale cannot overwrite the newest one.
  const update = pending.then(async () => {
    const keys = (await invoke<string[]>("native_menu_label_keys")) as MessageId[];
    const labels: Record<string, string> = {};
    for (const key of keys) {
      labels[key] = t(key, {}, locale, pseudoLocale);
    }
    if (account) {
      if (keys.includes("menu.signOut")) {
        labels["menu.signOut"] = t("accountMenu.signOutAccount", { account }, locale, pseudoLocale);
      }
      if (keys.includes("menu.accountSettings")) {
        labels["menu.accountSettings"] = t(
          "accountMenu.accountSettingsFor",
          { account },
          locale,
          pseudoLocale
        );
      }
    }
    await invoke("set_native_menu_labels", { labels });
  });
  pending = update.catch(() => undefined);
  return update;
}
