import { ModalDialog } from "../ModalDialog";

/**
 * #2096: the whole text of an attachment, read from Core's media cache.
 *
 * It replaces nothing: the card's Download tile still saves to the user's
 * Downloads folder, and this dialog only shows what the cache already holds.
 * `pre` keeps the file's own line breaks and indentation, which is the point of
 * reading source or log text without an editor.
 */
export function MediaTextPreviewDialog({
  filename,
  text,
  onClose
}: {
  filename: string;
  text: string;
  onClose: () => void;
}) {
  return (
    <ModalDialog
      className="media-text-preview-dialog"
      title={filename}
      onClose={onClose}
    >
      <pre className="media-text-preview-body" dir="auto" tabIndex={0}>
        {text}
      </pre>
    </ModalDialog>
  );
}
