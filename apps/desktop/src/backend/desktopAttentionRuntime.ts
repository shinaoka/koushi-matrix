import type { DesktopAttentionPort } from "./desktopAttentionPort";
import { isTauriRuntime } from "./runtimeEnvironment";
import { createTauriDesktopAttentionPort } from "./tauri/desktopAttentionPort";

export const desktopAttentionPort: DesktopAttentionPort | null = isTauriRuntime()
  ? createTauriDesktopAttentionPort()
  : null;
