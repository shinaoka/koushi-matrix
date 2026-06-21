import { useState } from "react";
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

  return (
    <span className="trust-help">
      <button
        className="trust-help-button"
        type="button"
        aria-label={t("help.userTrust.explain")}
        title={t("help.userTrust.explain")}
        onClick={() => setOpen((value) => !value)}
      >
        <HelpCircle size={13} aria-hidden="true" />
      </button>
      {open ? (
        <span className="trust-help-popover" role="dialog" aria-label={title}>
          <strong>{title}</strong>
          <span>{body}</span>
          <a href="docs/help/user-trust-model.md">{t("help.learnMore")}</a>
        </span>
      ) : null}
    </span>
  );
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
