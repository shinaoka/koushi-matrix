// @vitest-environment jsdom

import { createRef } from "react";
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AuthScreen, RecoveryPanel } from "./auth";
import type { DesktopSnapshot } from "../domain/types";
import { setActiveLocaleProfile } from "../i18n/messages";

describe("AuthScreen", () => {
  beforeEach(() => {
    setActiveLocaleProfile("en", "none");
  });

  afterEach(() => {
    cleanup();
    setActiveLocaleProfile("en", "none");
  });

  it("explains that the username field expects a localpart", () => {
    render(
      <AuthScreen
        deviceName="Koushi test"
        homeserver="matrix.org"
        isBusy={false}
        passwordFilled={true}
        passwordInputRef={createRef<HTMLInputElement>()}
        snapshot={snapshot({ session: { kind: "signedOut" } })}
        username=""
        onDeviceNameChange={vi.fn()}
        onDiscoverLoginMethods={vi.fn()}
        onHomeserverChange={vi.fn()}
        onPasswordPresenceChange={vi.fn()}
        onSubmit={vi.fn()}
        onUsernameChange={vi.fn()}
      />,
    );

    expect(screen.getByLabelText("Username").getAttribute("placeholder")).toBe("alice");
    expect(
      screen.getByText("Enter only the localpart. Do not include @ or the server name."),
    ).toBeTruthy();
  });

  it("adds a localpart hint to login failures", () => {
    render(
      <AuthScreen
        deviceName="Koushi test"
        homeserver="matrix.org"
        isBusy={false}
        passwordFilled={true}
        passwordInputRef={createRef<HTMLInputElement>()}
        snapshot={snapshot({
          session: { kind: "signedOut" },
          errors: [
            {
              code: "login_failed",
              message: "Login failed",
              recoverable: true,
            },
          ],
        })}
        username="@hiroshi.shinaoka:matrix.org"
        onDeviceNameChange={vi.fn()}
        onDiscoverLoginMethods={vi.fn()}
        onHomeserverChange={vi.fn()}
        onPasswordPresenceChange={vi.fn()}
        onSubmit={vi.fn()}
        onUsernameChange={vi.fn()}
      />,
    );

    expect(screen.getByRole("alert").textContent).toContain("Login failed");
    expect(screen.getByRole("alert").textContent).toContain(
      "For @alice:matrix.org, enter alice here and keep matrix.org in Homeserver.",
    );
  });
});

describe("RecoveryPanel", () => {
  beforeEach(() => {
    setActiveLocaleProfile("en", "none");
  });

  afterEach(() => {
    cleanup();
    setActiveLocaleProfile("en", "none");
  });

  it("does not show a stale login failure on the recovery screen", () => {
    render(
      <RecoveryPanel
        isBusy={false}
        secretFilled={false}
        secretInputRef={createRef<HTMLInputElement>()}
        snapshot={snapshot({
          session: {
            kind: "needsRecovery",
            user_id: "@hiroshi.shinaoka.test:matrix.org",
            recovery_methods: ["recoveryKey"],
          },
          errors: [
            {
              code: "login_failed",
              message: "Login failed",
              recoverable: true,
            },
          ],
        })}
        onSecretPresenceChange={vi.fn()}
        onSubmit={vi.fn()}
      />,
    );

    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("still shows recovery failures on the recovery screen", () => {
    render(
      <RecoveryPanel
        isBusy={false}
        secretFilled={false}
        secretInputRef={createRef<HTMLInputElement>()}
        snapshot={snapshot({
          session: {
            kind: "needsRecovery",
            user_id: "@hiroshi.shinaoka.test:matrix.org",
            recovery_methods: ["recoveryKey"],
          },
          errors: [
            {
              code: "e2ee_recovery_failed",
              message: "Recovery failed",
              recoverable: true,
            },
          ],
        })}
        onSecretPresenceChange={vi.fn()}
        onSubmit={vi.fn()}
      />,
    );

    expect(screen.getByRole("alert").textContent).toContain("Recovery failed");
  });
});

function snapshot({
  session,
  errors = [],
}: {
  session: Record<string, unknown>;
  errors?: Array<{ code: string; message: string; recoverable: boolean }>;
}): DesktopSnapshot {
  return {
    state: {
      domain: {
        auth: { kind: "unknown" },
        session,
      },
      ui: {
        errors,
      },
    },
  } as unknown as DesktopSnapshot;
}
