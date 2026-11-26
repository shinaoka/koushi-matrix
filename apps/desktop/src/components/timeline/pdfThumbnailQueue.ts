/**
 * Thumbnail jobs run one at a time, so a timeline that scrolls past many PDFs
 * holds one document's bytes, worker and canvas at once instead of one per card.
 * A job whose card unmounted before its turn is skipped without starting.
 */
let tail: Promise<unknown> = Promise.resolve();

export function runPdfThumbnailJob<T>(
  job: () => Promise<T>,
  isCancelled: () => boolean
): Promise<T | null> {
  const run = tail.then(() => (isCancelled() ? null : job()));
  tail = run.catch(() => undefined);
  return run;
}
