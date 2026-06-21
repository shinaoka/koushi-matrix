import { useCallback, useEffect, useState } from "react";

export const AVATAR_IMAGE_RETRY_DELAY_MS = 10_000;

export function useRecoverableImageSource(sourceUrl: string | null): {
  displaySourceUrl: string | null;
  onImageError: () => void;
  onImageLoad: () => void;
} {
  const [failedSourceUrl, setFailedSourceUrl] = useState<string | null>(null);

  useEffect(() => {
    setFailedSourceUrl(null);
  }, [sourceUrl]);

  useEffect(() => {
    if (!failedSourceUrl) {
      return undefined;
    }

    const retry = () => {
      setFailedSourceUrl((current) => (current === failedSourceUrl ? null : current));
    };
    const retryWhenVisible = () => {
      if (document.visibilityState === "visible") {
        retry();
      }
    };

    const timer = window.setTimeout(retry, AVATAR_IMAGE_RETRY_DELAY_MS);
    window.addEventListener("focus", retry);
    window.addEventListener("online", retry);
    document.addEventListener("visibilitychange", retryWhenVisible);
    return () => {
      window.clearTimeout(timer);
      window.removeEventListener("focus", retry);
      window.removeEventListener("online", retry);
      document.removeEventListener("visibilitychange", retryWhenVisible);
    };
  }, [failedSourceUrl]);

  const onImageError = useCallback(() => {
    if (sourceUrl) {
      setFailedSourceUrl(sourceUrl);
    }
  }, [sourceUrl]);

  const onImageLoad = useCallback(() => {
    setFailedSourceUrl((current) => (current === sourceUrl ? null : current));
  }, [sourceUrl]);

  return {
    displaySourceUrl: sourceUrl && sourceUrl !== failedSourceUrl ? sourceUrl : null,
    onImageError,
    onImageLoad
  };
}
