import type { DesktopAttentionPort } from "./desktopAttentionPort";
import { isTauriRuntime } from "./runtimeEnvironment";
import { createTauriDesktopAttentionPort } from "./tauri/desktopAttentionPort";

export function desktopAttentionPortForAccount(accountTabId?: string): DesktopAttentionPort | null {
  return isTauriRuntime() ? createTauriDesktopAttentionPort(accountTabId) : null;
}
