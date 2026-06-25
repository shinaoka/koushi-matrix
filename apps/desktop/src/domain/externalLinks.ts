import { openUrl } from "@tauri-apps/plugin-opener";

export function toExternalHttpUrl(rawUrl: string | null | undefined): string | null {
  if (!rawUrl) {
    return null;
  }
  try {
    const url = new URL(rawUrl);
    if (url.protocol !== "http:" && url.protocol !== "https:") {
      return null;
    }
    return url.toString();
  } catch {
    return null;
  }
}

export async function openExternalHttpUrl(rawUrl: string): Promise<void> {
  const url = toExternalHttpUrl(rawUrl);
  if (!url) {
    return;
  }
  try {
    await openUrl(url);
  } catch {
    window.open(url, "_blank", "noopener,noreferrer");
  }
}
