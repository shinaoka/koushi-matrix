// @vitest-environment jsdom

import { createRef } from "react";
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import koushiLogoUrl from "../assets/koushi-logo.svg";
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
        onStartOidcLogin={vi.fn()}
        onSubmit={vi.fn()}
        onUsernameChange={vi.fn()}
      />,
    );

    expect(screen.getByLabelText("Username").getAttribute("placeholder")).toBe("alice");
    expect(
      screen.getByText("Enter only the localpart. Do not include @ or the server name."),
    ).toBeTruthy();
  });

  it("keeps SSO launch errors inside the visible auth panel", () => {
    render(
      <AuthScreen
        deviceName="Koushi test"
        homeserver="matrix.org"
        isBusy={false}
        passwordFilled={false}
        passwordInputRef={createRef<HTMLInputElement>()}
        snapshot={snapshot({ session: { kind: "signedOut" } })}
        transportError="Could not open the browser for single sign-on"
        username=""
        onDeviceNameChange={vi.fn()}
        onDiscoverLoginMethods={vi.fn()}
        onHomeserverChange={vi.fn()}
        onPasswordPresenceChange={vi.fn()}
        onStartOidcLogin={vi.fn()}
        onSubmit={vi.fn()}
        onUsernameChange={vi.fn()}
      />,
    );

    const alert = screen.getByRole("alert");
    expect(alert.textContent).toContain("Could not open the browser");
    expect(alert.closest(".auth-panel")).toBeTruthy();
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
        onStartOidcLogin={vi.fn()}
        onSubmit={vi.fn()}
        onUsernameChange={vi.fn()}
      />,
    );

    expect(screen.getByRole("alert").textContent).toContain("Login failed");
    expect(screen.getByRole("alert").textContent).toContain(
      "For @alice:matrix.org, enter alice here and keep matrix.org in Homeserver.",
    );
  });

  it("shows sync auth failures on the sign-in screen", () => {
    render(
      <AuthScreen
        deviceName="Koushi test"
        homeserver="matrix.org"
        isBusy={false}
        passwordFilled={true}
        passwordInputRef={createRef<HTMLInputElement>()}
        snapshot={snapshot({
          session: { kind: "locked" },
          errors: [
            {
              code: "sync_auth_required",
              message: "sign-in required",
              recoverable: true,
            },
          ],
        })}
        username="hiroshi.shinaoka"
        onDeviceNameChange={vi.fn()}
        onDiscoverLoginMethods={vi.fn()}
        onHomeserverChange={vi.fn()}
        onPasswordPresenceChange={vi.fn()}
        onStartOidcLogin={vi.fn()}
        onSubmit={vi.fn()}
        onUsernameChange={vi.fn()}
      />,
    );

    expect(screen.getByText("Session locked")).toBeTruthy();
    expect(screen.getByRole("alert").textContent).toContain("sign-in required");
  });

  it("renders locked sessions with password and discovered OIDC reauthentication", () => {
    const onStartOidcLogin = vi.fn();
    render(
      <AuthScreen
        deviceName="Koushi test"
        homeserver=""
        isBusy={false}
        passwordFilled={true}
        passwordInputRef={createRef<HTMLInputElement>()}
        snapshot={snapshot({
          session: {
            kind: "locked",
            homeserver: "https://matrix.org",
            user_id: "@alice:matrix.org",
            device_id: "DEVICE",
          },
          auth: {
            kind: "ready",
            homeserver: "https://matrix.org",
            flows: [
              {
                kind: "oidc",
                delegated_oidc_compatibility: true,
                display_name: "Continue with provider",
              },
            ],
            delegated: { registration_url: null },
          },
        })}
        username=""
        onDeviceNameChange={vi.fn()}
        onDiscoverLoginMethods={vi.fn()}
        onHomeserverChange={vi.fn()}
        onPasswordPresenceChange={vi.fn()}
        onStartOidcLogin={onStartOidcLogin}
        onSubmit={vi.fn()}
        onUsernameChange={vi.fn()}
      />,
    );

    expect(screen.getByText("@alice:matrix.org")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Check login methods" })).toBeNull();
    expect(screen.queryByLabelText("Username")).toBeNull();
    expect(screen.queryByText("Device name")).toBeNull();
    expect(screen.getByLabelText("Password")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Continue" }).hasAttribute("disabled")).toBe(true);
    screen.getByRole("button", { name: "Continue with provider" }).click();
    expect(onStartOidcLogin).toHaveBeenCalledTimes(1);
  });

  it("offers OIDC login when discovery reports an OIDC flow", () => {
    const onStartOidcLogin = vi.fn();
    render(
      <AuthScreen
        deviceName="Koushi test"
        homeserver="matrix.org"
        isBusy={false}
        passwordFilled={false}
        passwordInputRef={createRef<HTMLInputElement>()}
        snapshot={snapshot({
          session: { kind: "signedOut" },
          auth: {
            kind: "ready",
            homeserver: "https://matrix.org",
            flows: [
              {
                kind: "oidc",
                delegated_oidc_compatibility: true,
                display_name: "Continue with provider",
              },
            ],
            delegated: {
              registration_url: "https://auth.example.test/register",
            },
          },
        })}
        username=""
        onDeviceNameChange={vi.fn()}
        onDiscoverLoginMethods={vi.fn()}
        onHomeserverChange={vi.fn()}
        onPasswordPresenceChange={vi.fn()}
        onStartOidcLogin={onStartOidcLogin}
        onSubmit={vi.fn()}
        onUsernameChange={vi.fn()}
      />,
    );

    const button = screen.getByRole("button", { name: "Continue with provider" });
    button.click();

    expect(onStartOidcLogin).toHaveBeenCalledTimes(1);
    expect(screen.getByLabelText("Password").hasAttribute("disabled")).toBe(true);
    expect(screen.getByRole("link", { name: "Create account" }).getAttribute("href")).toBe(
      "https://auth.example.test/register",
    );
  });

  it.each([
    ["signed-out login", { kind: "signedOut" } as const],
    [
      "locked-session reauthentication",
      {
        kind: "locked",
        homeserver: "https://matrix.example.test",
        user_id: "@alice:example.test",
        device_id: "DEVICE",
      } as const,
    ],
  ])("renders the Koushi brand asset instead of a generic hash on %s", (_label, session) => {
    const { container } = render(
      <AuthScreen
        deviceName="Koushi test"
        homeserver="matrix.example.test"
        isBusy={false}
        passwordFilled={false}
        passwordInputRef={createRef<HTMLInputElement>()}
        snapshot={snapshot({ session })}
        username=""
        onDeviceNameChange={vi.fn()}
        onDiscoverLoginMethods={vi.fn()}
        onHomeserverChange={vi.fn()}
        onPasswordPresenceChange={vi.fn()}
        onStartOidcLogin={vi.fn()}
        onSubmit={vi.fn()}
        onUsernameChange={vi.fn()}
      />,
    );

    const mark = container.querySelector(".auth-brand .auth-mark");
    expect(mark).not.toBeNull();
    expect(mark?.querySelector("svg.lucide-hash")).toBeNull();
    const logo = mark?.querySelector("img.auth-logo");
    expect(logo).not.toBeNull();
    expect(logo?.getAttribute("src")).toBe(koushiLogoUrl);
    // The adjacent "Koushi" heading already names the brand, so the logo is
    // decorative and must not add a redundant screen-reader announcement.
    expect(logo?.getAttribute("alt")).toBe("");
    expect(screen.queryByRole("img")).toBeNull();
    expect(screen.getByRole("heading", { level: 1, name: "Koushi" })).toBeTruthy();
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
  auth = { kind: "unknown" },
  errors = [],
}: {
  session: Record<string, unknown>;
  auth?: Record<string, unknown>;
  errors?: Array<{ code: string; message: string; recoverable: boolean }>;
}): DesktopSnapshot {
  return {
    state: {
      domain: {
        auth,
        session,
      },
      ui: {
        errors,
      },
    },
  } as unknown as DesktopSnapshot;
}
