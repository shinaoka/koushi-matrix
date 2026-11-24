// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";

import { SecureBackupSendNotice } from "./SecureBackupSendNotice";
import { t } from "../i18n/messages";

describe("SecureBackupSendNotice", () => {
  afterEach(() => {
    cleanup();
  });

  test("puts the way out of the block beside the reason", () => {
    const onRetry = vi.fn();
    const onOpenSettings = vi.fn();
    render(<SecureBackupSendNotice onRetry={onRetry} onOpenSettings={onOpenSettings} />);

    // The reason is the live region and the actions are its siblings, so a
    // status announcement never contains the buttons it announced.
    // `readable-provisional-shell.spec.ts` reads this class, its role and its
    // exact text as the read-only contract, so pin them here too: adding the
    // actions must never move them off this element.
    const region = screen.getByRole("status");
    expect(region.className).toBe("composer-read-only-notice");
    expect(region.textContent).toBe(t("composer.sendBlockedSecureBackup"));
    expect(region.querySelector("button")).toBeNull();

    fireEvent.click(
      screen.getByRole("button", { name: t("composer.sendBlockedSecureBackupRecheck") })
    );
    expect(onRetry).toHaveBeenCalledTimes(1);

    fireEvent.click(
      screen.getByRole("button", { name: t("composer.sendBlockedSecureBackupSettings") })
    );
    expect(onOpenSettings).toHaveBeenCalledTimes(1);
  });

  test("refuses a second re-check while the first is settling", () => {
    const onRetry = vi.fn();
    render(<SecureBackupSendNotice retrying onRetry={onRetry} onOpenSettings={() => undefined} />);

    const recheck = screen.getByRole("button", {
      name: t("composer.sendBlockedSecureBackupRecheck")
    });
    expect((recheck as HTMLButtonElement).disabled).toBe(true);
  });

  test("offers no re-check when the recorded cause is not retryable", () => {
    render(
      <SecureBackupSendNotice
        retryable={false}
        onRetry={() => undefined}
        onOpenSettings={() => undefined}
      />
    );

    expect(
      screen.queryByRole("button", { name: t("composer.sendBlockedSecureBackupRecheck") })
    ).toBeNull();
    // The way out that does help — the settings page that shows this gate —
    // stays.
    expect(
      screen.getByRole("button", { name: t("composer.sendBlockedSecureBackupSettings") })
    ).toBeTruthy();
  });
});
