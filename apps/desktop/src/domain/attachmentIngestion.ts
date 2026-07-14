import type { ComposerTarget, StageUploadBytesRequestItem } from "./types";

type AttachmentTransfer = {
  files: ArrayLike<File>;
  items: ArrayLike<{ kind: string }>;
  types: ArrayLike<string>;
};

export function attachmentTransferHasFiles(transfer: AttachmentTransfer): boolean {
  return (
    Array.from(transfer.types).includes("Files") ||
    Array.from(transfer.items).some((item) => item.kind === "file") ||
    transfer.files.length > 0
  );
}

export function filesFromAttachmentTransfer(transfer: AttachmentTransfer): File[] {
  return attachmentTransferHasFiles(transfer) ? Array.from(transfer.files) : [];
}

export async function ingestAttachmentFiles(
  files: Iterable<File>,
  ingest: (files: File[]) => void | Promise<void>
): Promise<boolean> {
  const orderedFiles = Array.from(files);
  if (orderedFiles.length === 0) {
    return false;
  }
  await ingest(orderedFiles);
  return true;
}

export async function stageAttachmentFiles(
  target: ComposerTarget,
  files: Iterable<File>,
  startPosition: number,
  createStagedId: (position: number) => string,
  stage: (target: ComposerTarget, items: StageUploadBytesRequestItem[]) => Promise<unknown>
): Promise<boolean> {
  const orderedFiles = Array.from(files);
  if (orderedFiles.length === 0) {
    return false;
  }
  const items = await Promise.all(
    orderedFiles.map(async (file, index) => {
      const position = startPosition + index + 1;
      return {
        stagedId: createStagedId(position),
        position,
        filename: file.name || "attachment",
        mimeType: file.type || "application/octet-stream",
        bytes: Array.from(new Uint8Array(await file.arrayBuffer()))
      };
    })
  );
  await stage(target, items);
  return true;
}
