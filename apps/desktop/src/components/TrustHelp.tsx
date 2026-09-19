import { useRef, useState, type RefObject } from "react";
import { FloatingLayer, floatingPlacementStyle, useFloatingPlacement } from "./floatingLayer";
import { HelpCircle } from "lucide-react";

import { t } from "../i18n/messages";
import type { UserTrustState } from "../domain/types";

export function TrustHelpButton({
  body,
  title
}: {
  body: string;
  title: string;
}) {
  const [open, setOpen] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);

  return (
    <span className="trust-help">
      <button
        ref={trigger}
        className="trust-help-button"
        type="button"
        aria-label={t("help.userTrust.explain")}
        title={t("help.userTrust.explain")}
        aria-expanded={open}
        onKeyDown={(event) => {
          if (event.key === "Escape" && open && !event.nativeEvent.isComposing) {
            event.preventDefault(); event.stopPropagation(); setOpen(false);
          }
        }}
        onClick={() => setOpen((value) => !value)}
      >
        <HelpCircle size={13} aria-hidden="true" />
      </button>
      {open ? (
        <TrustHelpPopup trigger={trigger} title={title} body={body} onClose={() => setOpen(false)} />
      ) : null}
    </span>
  );
}

function TrustHelpPopup({ trigger, title, body, onClose }: {
  trigger: RefObject<HTMLButtonElement | null>; title: string; body: string; onClose: () => void;
}) {
  const placement = useFloatingPlacement({ anchorRef: trigger, placement: "below", align: "start", inlineSize: 300, blockSize: 220 });
  return <FloatingLayer><span className="trust-help-popover" role="dialog" aria-label={title}
    style={floatingPlacementStyle(placement)}
    onKeyDown={(event) => {
      if (event.key === "Escape" && !event.nativeEvent.isComposing) {
        event.preventDefault(); event.stopPropagation(); onClose(); trigger.current?.focus();
      }
    }}>
    <strong>{title}</strong><span>{body}</span>
    <a href="docs/help/user-trust-model.md">{t("help.learnMore")}</a>
  </span></FloatingLayer>;
}

export function UserTrustChip({ state }: { state?: UserTrustState | null }) {
  const normalized = state ?? { kind: "unverified" as const };

  return (
    <span className={`user-trust-chip ${normalized.kind}`}>
      <span>{userTrustLabel(normalized)}</span>
      <TrustHelpButton
        title={userTrustHelpTitle(normalized)}
        body={userTrustHelpBody(normalized)}
      />
    </span>
  );
}

export function userTrustLabel(state: UserTrustState): string {
  switch (state.kind) {
    case "identityReset":
      return t("trust.userIdentityReset");
    case "unverified":
      return t("trust.userUnverified");
    case "verified":
      return t("trust.userVerified");
  }
}

function userTrustHelpTitle(state: UserTrustState): string {
  switch (state.kind) {
    case "identityReset":
      return t("help.userTrust.identityResetTitle");
    case "unverified":
      return t("help.userTrust.unverifiedTitle");
    case "verified":
      return t("help.userTrust.verifiedTitle");
  }
}

function userTrustHelpBody(state: UserTrustState): string {
  switch (state.kind) {
    case "identityReset":
      return t("help.userTrust.identityResetBody");
    case "unverified":
      return t("help.userTrust.unverifiedBody");
    case "verified":
      return t("help.userTrust.verifiedBody");
  }
}
