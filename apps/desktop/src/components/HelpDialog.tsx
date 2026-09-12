import { useState } from "react";
import { Copy } from "lucide-react";
import { t } from "../i18n/messages";
import { ModalDialog } from "./ModalDialog";

export const HELP_REPOSITORY_URL = "https://github.com/shinaoka/koushi-matrix";

export function HelpContent() {
  const [copyState, setCopyState] = useState<"idle" | "copying" | "copied" | "failed">("idle");
  async function copyUrl() {
    setCopyState("copying");
    try {
      if (!navigator.clipboard) throw new Error("clipboard unavailable");
      await navigator.clipboard.writeText(HELP_REPOSITORY_URL);
      setCopyState("copied");
    } catch {
      setCopyState("failed");
    }
  }
  return <div className="help-content">
    <p>{t("help.askAi")}</p>
    <p className="help-repository-url" dir="ltr">{HELP_REPOSITORY_URL}</p>
    <button type="button" className="dialog-button is-primary" disabled={copyState === "copying"} onClick={() => void copyUrl()}>
      <Copy size={16} aria-hidden="true" /> {t("help.copyRepositoryUrl")}
    </button>
    <p role="status" className="help-copy-status">
      {copyState === "copied" ? t("help.urlCopied") : copyState === "failed" ? t("help.copyFailed") : ""}
    </p>
  </div>;
}

export function HelpDialog({ onClose }: { onClose: () => void }) {
  return <ModalDialog title={t("help.title")} className="help-modal" onClose={onClose}><HelpContent /></ModalDialog>;
}
