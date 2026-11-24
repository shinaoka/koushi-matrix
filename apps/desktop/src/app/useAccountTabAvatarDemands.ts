import { useEffect, useRef } from "react";

import { createTauriAccountTabAvatarPort } from "../backend/tauri/desktopAccountTabAvatarPort";
import { AVATAR_THUMBNAIL_DOWNLOADS_ENABLED } from "../domain/avatarThumbnails";
import type { AccountTabsSnapshot } from "../domain/types";

/**
 * #2059: demand a background account's avatar thumbnail for its tab.
 *
 * A tab only shows an image once its snapshot carries a Ready reference, and
 * nothing demanded one for an account the reader is not in. The selected
 * account stays with its own transport's demand; this covers the rest.
 */
export function useAccountTabAvatarDemands(accountTabs: AccountTabsSnapshot | null): void {
  const demandedRef = useRef(new Set<string>());
  useEffect(() => {
    const port = createTauriAccountTabAvatarPort();
    const demanded = demandedRef.current;
    const wanted = new Set<string>();
    if (port && AVATAR_THUMBNAIL_DOWNLOADS_ENABLED) {
      for (const tab of accountTabs?.tabs ?? []) {
        // The selected account's own avatar comes from its timeline, on that
        // account's transport; asking again here would add a second owner.
        if (tab.id === accountTabs?.selectedTabId) continue;
        // A tab without a session yet has no client to fetch with, and Core drops
        // the update, so its one demand would be spent for nothing.
        if (tab.status !== "ready") continue;
        if (!tab.avatarMxcUri || tab.avatarSourceRef) continue;
        const key = `${tab.id}\u0000${tab.avatarMxcUri}`;
        wanted.add(key);
        if (demanded.has(key)) continue;
        demanded.add(key);
        void port.downloadThumbnail(tab.id, tab.avatarMxcUri).catch(() => {
          // The adapter refused before Core owned it; the next snapshot retries.
          demanded.delete(key);
        });
      }
    }
    for (const key of demanded) {
      if (!wanted.has(key)) demanded.delete(key);
    }
  }, [accountTabs]);
}
