// @vitest-environment jsdom

import { createRef } from "react";
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import koushiLogoUrl from "../assets/koushi-logo.svg";
import { AuthScreen, RecoveryPanel } from "./auth";
import type { DesktopSnapshot } from "../domain/types";
import { setActiveLocaleProfile } from "../i18n/messages";

type AuthScreenProps = Parameters<typeof AuthScreen>[0];

function renderAuth(overrides: Partial<AuthScreenProps> = {}) {
  const props: AuthScreenProps = {
    deviceName: "Koushi test",
    effectiveServer: "matrix.org",
    isBusy: false,
    matrixId: "",
    passwordFilled: false,
    passwordInputRef: createRef<HTMLInputElement>(),
    serverOverride: null,
    snapshot: snapshot({ session: { kind: "signedOut" } }),
    onDeviceNameChange: vi.fn(),
    onDiscoverLoginMethods: vi.fn(),
    onMatrixIdChange: vi.fn(),
    onPasswordPresenceChange: vi.fn(),
    onServerOverrideChange: vi.fn(),
    onStartOidcLogin: vi.fn(),
    onSubmit: vi.fn(),
    ...overrides,
  };
  return { props, ...render(<AuthScreen {...props} />) };
}

function oidcAuth(homeserver: string, flows: Array<"oidc" | "password">) {
  return {
    kind: "ready",
    homeserver,
    flows: flows.map((kind) =>
      kind === "oidc"
        ? { kind, delegated_oidc_compatibility: true, display_name: "Example ID" }
        : { kind, delegated_oidc_compatibility: false, display_name: null },
    ),
    delegated: { registration_url: "https://auth.example.test/register" },
  } as const;
}

describe("AuthScreen", () => {
  beforeEach(() => {
    setActiveLocaleProfile("en", "none");
  });

  afterEach(() => {
    cleanup();
    setActiveLocaleProfile("en", "none");
  });

  it("asks for one Matrix ID and derives the server instead of a separate homeserver field", () => {
    const { container, props } = renderAuth({
      effectiveServer: "example.org",
      matrixId: "@alice:example.org",
    });

    expect(screen.getByLabelText("Matrix ID").getAttribute("placeholder")).toBe(
      "@alice:matrix.org",
    );
    expect(container.querySelector('input[name="homeserver"]')).toBeNull();
    expect(screen.getByTestId("auth-server-summary").textContent).toContain("example.org");

    screen.getByRole("button", { name: "Change server" }).click();
    expect(props.onServerOverrideChange).toHaveBeenCalledWith("example.org");
  });

  it("keeps an explicit server path that can return to the Matrix ID server", () => {
    const { container, props } = renderAuth({
      effectiveServer: "https://hs.example.net",
      serverOverride: "https://hs.example.net",
    });

    const homeserver = container.querySelector<HTMLInputElement>('input[name="homeserver"]');
    expect(homeserver?.value).toBe("https://hs.example.net");
    screen.getByRole("button", { name: "Use the server from my Matrix ID" }).click();
    expect(props.onServerOverrideChange).toHaveBeenCalledWith(null);
  });

  it("puts single sign-on first and password sign-in behind a divider", () => {
    const { container, props } = renderAuth({
      snapshot: snapshot({
        session: { kind: "signedOut" },
        auth: oidcAuth("matrix.org", ["oidc", "password"]),
      }),
    });

    const sso = screen.getByRole("button", { name: "Continue with Example ID" });
    expect(sso.className).toBe("auth-sso-button");
    const password = screen.getByLabelText("Password");
    expect(
      sso.compareDocumentPosition(password) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(screen.getByRole("separator").textContent).toBe("or sign in with a password");
    expect(container.querySelector("button.auth-submit")?.className).toContain(
      "auth-submit-secondary",
    );
    expect(screen.getByRole("link", { name: "Create account" }).getAttribute("href")).toBe(
      "https://auth.example.test/register",
    );

    sso.click();
    expect(props.onStartOidcLogin).toHaveBeenCalledTimes(1);
  });

  it("hides password sign-in when the server only offers single sign-on", () => {
    renderAuth({
      snapshot: snapshot({
        session: { kind: "signedOut" },
        auth: oidcAuth("matrix.org", ["oidc"]),
      }),
    });

    expect(screen.getByRole("button", { name: "Continue with Example ID" })).toBeTruthy();
    expect(screen.queryByLabelText("Password")).toBeNull();
    expect(screen.getByText("This server does not offer password sign-in.")).toBeTruthy();
  });

  it("ignores login methods discovered for a different server", () => {
    renderAuth({
      effectiveServer: "example.org",
      snapshot: snapshot({
        session: { kind: "signedOut" },
        auth: oidcAuth("matrix.org", ["oidc"]),
      }),
    });

    expect(screen.queryByRole("button", { name: "Continue with Example ID" })).toBeNull();
    expect(screen.getByLabelText("Password")).toBeTruthy();
  });

  it("offers a retry when login method discovery failed", () => {
    const { props } = renderAuth({
      snapshot: snapshot({
        session: { kind: "signedOut" },
        auth: { kind: "failed", homeserver: "matrix.org", failureKind: "network" },
      }),
    });

    expect(screen.getByText("Could not reach the homeserver")).toBeTruthy();
    screen.getByRole("button", { name: "Check login methods" }).click();
    expect(props.onDiscoverLoginMethods).toHaveBeenCalledTimes(1);
  });

  it("cancels an unfinished add-account sign-in only when offered", () => {
    const onCancel = vi.fn();
    renderAuth({ onCancel });
    screen.getByRole("button", { name: "Cancel" }).click();
    expect(onCancel).toHaveBeenCalledTimes(1);

    cleanup();
    renderAuth();
    expect(screen.queryByRole("button", { name: "Cancel" })).toBeNull();
  });

  it("keeps SSO launch errors inside the visible auth panel", () => {
    renderAuth({ transportError: "Could not open the browser for single sign-on" });

    const alert = screen.getByRole("alert");
    expect(alert.textContent).toContain("Could not open the browser");
    expect(alert.closest(".auth-panel")).toBeTruthy();
  });

  it("adds a Matrix ID hint to login failures", () => {
    renderAuth({
      matrixId: "@alice:matrix.org",
      passwordFilled: true,
      snapshot: snapshot({
        session: { kind: "signedOut" },
        errors: [
          {
            code: "login_failed",
            message: "Login failed",
            recoverable: true,
          },
        ],
      }),
    });

    expect(screen.getByRole("alert").textContent).toContain("Login failed");
    expect(screen.getByRole("alert").textContent).toContain(
      "Check your Matrix ID, for example @alice:matrix.org, and your password.",
    );
  });

  it("shows sync auth failures on the sign-in screen", () => {
    renderAuth({
      passwordFilled: true,
      snapshot: snapshot({
        session: { kind: "locked" },
        errors: [
          {
            code: "sync_auth_required",
            message: "sign-in required",
            recoverable: true,
          },
        ],
      }),
    });

    expect(screen.getByText("Session locked")).toBeTruthy();
    expect(screen.getByRole("alert").textContent).toContain("sign-in required");
  });

  it("renders locked sessions with password and discovered OIDC reauthentication", () => {
    const onStartOidcLogin = vi.fn();
    renderAuth({
      effectiveServer: "",
      passwordFilled: true,
      snapshot: snapshot({
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
      }),
      onStartOidcLogin,
    });

    expect(screen.getByText("@alice:matrix.org")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Check login methods" })).toBeNull();
    expect(screen.queryByLabelText("Matrix ID")).toBeNull();
    expect(screen.queryByText("Device name")).toBeNull();
    expect(screen.getByLabelText("Password")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Continue" }).hasAttribute("disabled")).toBe(true);
    screen.getByRole("button", { name: "Continue with provider" }).click();
    expect(onStartOidcLogin).toHaveBeenCalledTimes(1);
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
    const { container } = renderAuth({
      effectiveServer: "matrix.example.test",
      snapshot: snapshot({ session }),
    });

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
