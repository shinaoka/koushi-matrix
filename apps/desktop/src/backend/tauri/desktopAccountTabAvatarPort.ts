import { invoke } from "@tauri-apps/api/core";

import { isTauriRuntime } from "../runtimeEnvironment";

/**
 * #2059: fetch a background account's own avatar thumbnail for its tab.
 *
 * `download_avatar_thumbnail` already addresses the account by tab id, but the
 * per-account transports refuse to invoke for a tab that is not the selected one
 * (`isRendererSelectedAccountTabId` is that stale-transport guard, not an
 * authorisation boundary). Core owns dedupe, admission, retries and the terminal
 * state, so this only names the account and the media resource.
 */
export interface AccountTabAvatarPort {
  downloadThumbnail(accountTabId: string, mxcUri: string): Promise<void>;
}

export function createTauriAccountTabAvatarPort(): AccountTabAvatarPort | null {
  if (!isTauriRuntime()) return null;
  return {
    async downloadThumbnail(accountTabId, mxcUri) {
      await invoke("download_avatar_thumbnail", { accountTabId, mxcUri });
    }
  };
}
