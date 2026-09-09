import { useEffect, useState } from "react";
import type { DesktopApi } from "../backend/desktopApi";
import type { RoomAddressPreview } from "../domain/types";

/** Render a Rust projection of the current unsent draft, never an older response. */
export function useRoomAddressPreview(
  api: Pick<DesktopApi, "previewRoomAddress">,
  name: string,
  alias: string | null,
  account: string | null,
  enabled: boolean
): RoomAddressPreview | null {
  const [result, setResult] = useState<{
    name: string; alias: string | null; account: string | null; preview: RoomAddressPreview;
  } | null>(null);
  useEffect(() => {
    let current = true;
    if (enabled) {
      void api.previewRoomAddress(name, alias).then(preview => {
        if (current) setResult({ name, alias, account, preview });
      }).catch(() => {
        if (current) setResult(null);
      });
    }
    return () => { current = false; };
  }, [api, name, alias, account, enabled]);
  return enabled && result?.name === name && result.alias === alias && result.account === account
    ? result.preview : null;
}
