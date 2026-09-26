// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";

import {
  type AccountNotificationActions,
  EmailNotificationsSection,
  NotificationCategoriesSection
} from "./AccountNotificationsSections";
import type { AccountNotificationsSnapshot, AccountNotificationsState } from "../../domain/types";

function actions(): AccountNotificationActions {
  return {
    load: vi.fn(),
    setCategory: vi.fn(),
    setAccountPush: vi.fn(),
    requestEmailToken: vi.fn(),
    resendEmailToken: vi.fn(),
    confirmEmail: vi.fn(),
    submitUia: vi.fn(),
    cancelEmail: vi.fn(),
    enableEmail: vi.fn(),
    disableEmail: vi.fn()
  };
}

function snapshot(overrides: Partial<AccountNotificationsSnapshot> = {}): AccountNotificationsSnapshot {
  return {
    account_push_enabled: true,
    encrypted_event_push: false,
    categories: {
      direct_messages: "on",
      group_messages: "mixed",
      mentions_and_replies: "on",
      invites: "off"
    },
    email_management: "available",
    emails: [],
    unverified_email_pusher_count: 0,
    ...overrides
  };
}

function loaded(
  snap: AccountNotificationsSnapshot,
  extra: Partial<AccountNotificationsState> = {}
): AccountNotificationsState {
  return {
    load: { kind: "loaded" },
    snapshot: snap,
    pending_email: null,
    operation: { kind: "idle" },
    ...extra
  };
}

function renderEmail(state: AccountNotificationsState, handlers = actions()) {
  render(
    <EmailNotificationsSection
      state={state}
      actions={handlers}
      syncKey="session"
      accountManagementAvailable={false}
      onManageAccount={() => undefined}
    />
  );
  return handlers;
}

afterEach(cleanup);

describe("NotificationCategoriesSection", () => {
  test("renders the Rust category states and marks mixed without claiming ON", () => {
    const handlers = actions();
    render(<NotificationCategoriesSection state={loaded(snapshot())} actions={handlers} />);
    const dms = screen.getByRole("switch", { name: "Direct messages" });
    const group = screen.getByRole("switch", { name: "Group messages" });
    const invites = screen.getByRole("switch", { name: "Room invites" });
    expect(dms.getAttribute("aria-checked")).toBe("true");
    expect(group.getAttribute("aria-checked")).toBe("false");
    expect(group.getAttribute("data-state")).toBe("mixed");
    expect(within(group).getByText(/Set differently in another app/)).toBeTruthy();
    expect(invites.getAttribute("aria-checked")).toBe("false");

    // Rendering alone dispatches nothing.
    expect(handlers.setCategory).not.toHaveBeenCalled();

    fireEvent.click(group);
    expect(handlers.setCategory).toHaveBeenCalledWith("groupMessages", true);
    fireEvent.click(dms);
    expect(handlers.setCategory).toHaveBeenCalledWith("directMessages", false);
  });

  test("keeps the server value while a toggle is in flight", () => {
    const state = loaded(snapshot(), {
      operation: {
        kind: "working",
        request_id: 3,
        operation: { kind: "setCategory", category: "invites", enabled: true }
      }
    });
    render(<NotificationCategoriesSection state={state} actions={actions()} />);
    const invites = screen.getByRole("switch", { name: "Room invites" });
    expect(invites.getAttribute("aria-checked")).toBe("false");
    expect((invites as HTMLButtonElement).disabled).toBe(true);
  });

  test("offers recovery when another client silenced the account", () => {
    const handlers = actions();
    render(
      <NotificationCategoriesSection
        state={loaded(snapshot({ account_push_enabled: false }))}
        actions={handlers}
      />
    );
    fireEvent.click(screen.getByRole("button", { name: "Turn on" }));
    expect(handlers.setAccountPush).toHaveBeenCalledWith(true);
  });
});

describe("NotificationCategoriesSection caveats", () => {
  test("warns that encrypted-group mentions reach only this app while group is off", () => {
    render(
      <NotificationCategoriesSection
        state={loaded(
          snapshot({
            categories: {
              direct_messages: "on",
              group_messages: "off",
              mentions_and_replies: "on",
              invites: "on"
            }
          })
        )}
        actions={actions()}
      />
    );
    expect(screen.getByTestId("encrypted-group-caveat").textContent).toMatch(
      /encrypted group rooms your server cannot see mentions/
    );
  });

  test("warns that MSC4028 servers still push encrypted messages", () => {
    render(
      <NotificationCategoriesSection
        state={loaded(
          snapshot({
            encrypted_event_push: true,
            categories: {
              direct_messages: "on",
              group_messages: "off",
              mentions_and_replies: "on",
              invites: "on"
            }
          })
        )}
        actions={actions()}
      />
    );
    expect(screen.getByTestId("encrypted-group-caveat").textContent).toMatch(
      /pushes every encrypted message/
    );
  });

  test("no caveat while group messages are on; unavailable categories are disabled", () => {
    const handlers = actions();
    render(
      <NotificationCategoriesSection
        state={loaded(
          snapshot({
            categories: {
              direct_messages: "on",
              group_messages: "on",
              mentions_and_replies: "on",
              invites: "unavailable"
            }
          })
        )}
        actions={handlers}
      />
    );
    expect(screen.queryByTestId("encrypted-group-caveat")).toBeNull();
    const invites = screen.getByRole("switch", { name: "Room invites" }) as HTMLButtonElement;
    expect(invites.disabled).toBe(true);
    expect(within(invites).getByText("Not available on this server.")).toBeTruthy();
    fireEvent.click(invites);
    expect(handlers.setCategory).not.toHaveBeenCalled();
  });
});

describe("EmailNotificationsSection", () => {
  test("cannot enable email notifications before an address is verified", () => {
    renderEmail(loaded(snapshot()));
    const toggle = screen.getByRole("switch", { name: "Email notifications" });
    expect(toggle.getAttribute("aria-checked")).toBe("false");
    expect((toggle as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText("Add and verify an email address first")).toBeTruthy();
  });

  test("shows the active pusher target and turns it off", () => {
    const handlers = renderEmail(
      loaded(
        snapshot({
          emails: [
            { address: "one@example.invalid", notifications_active: false },
            { address: "two@example.invalid", notifications_active: true }
          ]
        })
      )
    );
    const toggle = screen.getByRole("switch", { name: "Email notifications" });
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    // With several addresses the picker shows the target; the switch only says On.
    expect(within(toggle).getByText("On")).toBeTruthy();
    const select = screen.getByTestId("email-notifications-target") as HTMLSelectElement;
    expect(select.value).toBe("two@example.invalid");
    fireEvent.change(select, { target: { value: "one@example.invalid" } });
    expect(handlers.enableEmail).toHaveBeenCalledWith("one@example.invalid");
    fireEvent.click(toggle);
    expect(handlers.disableEmail).toHaveBeenCalled();
  });

  test("enables a verified address and adds a new one through verification", () => {
    const handlers = renderEmail(
      loaded(snapshot({ emails: [{ address: "one@example.invalid", notifications_active: false }] }))
    );
    fireEvent.click(screen.getByRole("switch", { name: "Email notifications" }));
    expect(handlers.enableEmail).toHaveBeenCalledWith("one@example.invalid");

    fireEvent.click(screen.getByRole("button", { name: "Change" }));
    const input = screen.getByTestId("email-address-input");
    fireEvent.change(input, { target: { value: "new@example.invalid" } });
    fireEvent.click(screen.getByRole("button", { name: "Send verification email" }));
    expect(handlers.requestEmailToken).toHaveBeenCalledWith("new@example.invalid");
  });

  test("pending verification offers continue, resend, and password re-auth", () => {
    const handlers = actions();
    const pending = { address: "new@example.invalid", resend_count: 1 };
    const { rerender } = render(
      <EmailNotificationsSection
        state={loaded(snapshot(), { pending_email: pending })}
        actions={handlers}
        syncKey="session"
        accountManagementAvailable={false}
        onManageAccount={() => undefined}
      />
    );
    expect(screen.getByText(/We sent a verification email to new@example.invalid/)).toBeTruthy();
    expect(screen.getByText("Verification email sent again.")).toBeTruthy();
    expect(
      screen.getByRole("switch", { name: "Email notifications" }).getAttribute("aria-checked")
    ).toBe("false");
    fireEvent.click(screen.getByTestId("email-resend"));
    expect(handlers.resendEmailToken).toHaveBeenCalled();
    fireEvent.click(screen.getByTestId("email-confirm"));
    expect(handlers.confirmEmail).toHaveBeenCalled();

    rerender(
      <EmailNotificationsSection
        state={loaded(snapshot(), {
          pending_email: pending,
          operation: {
            kind: "awaitingUia",
            request_id: 7,
            flow_id: 7,
            operation: { kind: "confirmEmail" }
          }
        })}
        actions={handlers}
        syncKey="session"
        accountManagementAvailable={false}
        onManageAccount={() => undefined}
      />
    );
    const password = document.querySelector("input[type=password]") as HTMLInputElement;
    password.value = "synthetic-password";
    fireEvent.input(password);
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    expect(handlers.submitUia).toHaveBeenCalledWith(7, "synthetic-password");
  });

  test("explains unsupported servers and failures without showing ON", () => {
    renderEmail(
      loaded(snapshot({ email_management: "unsupported" }), {
        operation: {
          kind: "failed",
          request_id: 2,
          operation: { kind: "enableEmailNotifications" },
          failureKind: "unsupported"
        }
      })
    );
    expect(screen.getByTestId("email-unsupported")).toBeTruthy();
    expect(screen.getByTestId("email-notifications-error").textContent).toBe(
      "This server does not support this."
    );
    expect(screen.queryByTestId("email-add")).toBeNull();
    expect(
      screen.getByRole("switch", { name: "Email notifications" }).getAttribute("aria-checked")
    ).toBe("false");
  });
});
