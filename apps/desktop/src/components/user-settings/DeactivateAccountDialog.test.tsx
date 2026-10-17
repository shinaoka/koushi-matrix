// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";

import type { AccountManagementCapabilities, AccountManagementState } from "../../domain/types";
import { AccountManagementSection } from "./AccountManagementSection";
import { DeactivateAccountDialog } from "./DeactivateAccountDialog";

const session = {
  homeserver: "https://matrix.example.invalid",
  user_id: "@member:example.invalid",
  device_id: "SYNTHETIC"
};
const idle: AccountManagementState = { kind: "idle" };
const enabled: AccountManagementCapabilities = {
  change_password: { kind: "enabled" },
  deactivate_account: { kind: "enabled" }
};

function renderSection(
  onDeactivateAccount = vi.fn(),
  capabilities: AccountManagementCapabilities = enabled
) {
  render(
    <AccountManagementSection
      accountManagement={idle}
      accountManagementCapabilities={capabilities}
      accountManagementUrl={null}
      currentSession={session}
      onLoadAccountManagementCapabilities={vi.fn()}
      onChangePassword={vi.fn()}
      onDeactivateAccount={onDeactivateAccount}
      onManageAccount={vi.fn()}
      onSubmitAccountManagementUia={vi.fn()}
    />
  );
  return onDeactivateAccount;
}

function typePassword(value: string) {
  const field = screen.getByTestId("deactivate-account-password") as HTMLInputElement;
  field.value = value;
  fireEvent.input(field);
}

describe("account deactivation (#1503)", () => {
  afterEach(cleanup);

  test("the entry button only opens a dialog naming the target account", () => {
    const onDeactivate = renderSection();
    fireEvent.click(screen.getByTestId("deactivate-account-button"));
    fireEvent.click(screen.getByTestId("deactivate-account-button"));

    expect(onDeactivate).not.toHaveBeenCalled();
    expect(screen.getByTestId("deactivate-account-target").textContent).toBe(
      "@member:example.invalid"
    );
    expect(screen.getByTestId("deactivate-account-dialog")).toBeTruthy();
  });

  test("continue stays disabled until a password is entered", () => {
    renderSection();
    fireEvent.click(screen.getByTestId("deactivate-account-button"));
    const continueButton = screen.getByTestId("deactivate-account-continue") as HTMLButtonElement;
    expect(continueButton.disabled).toBe(true);

    typePassword("synthetic-password");
    expect(continueButton.disabled).toBe(false);
  });

  test("the final confirmation dispatches deactivation with the password", () => {
    const onDeactivate = renderSection();
    fireEvent.click(screen.getByTestId("deactivate-account-button"));
    fireEvent.click(screen.getByTestId("deactivate-erase-checkbox"));
    typePassword("synthetic-password");
    fireEvent.click(screen.getByTestId("deactivate-account-continue"));
    expect(onDeactivate).not.toHaveBeenCalled();

    fireEvent.click(screen.getByTestId("deactivate-account-confirm"));
    expect(onDeactivate).toHaveBeenCalledExactlyOnceWith(true, "synthetic-password");
  });

  test("a double click on Continue lands on Cancel, not on the destructive action", () => {
    const onDeactivate = renderSection();
    fireEvent.click(screen.getByTestId("deactivate-account-button"));
    typePassword("synthetic-password");
    const buttons = () =>
      Array.from(
        screen.getByRole("dialog").querySelectorAll<HTMLButtonElement>(".dialog-actions button")
      );
    const continueIndex = buttons().findIndex(
      (button) => button.dataset.testid === "deactivate-account-continue"
    );
    fireEvent.click(screen.getByTestId("deactivate-account-continue"));

    expect(buttons()[continueIndex]?.dataset.testid).toBe("deactivate-account-confirm-cancel");
    expect(onDeactivate).not.toHaveBeenCalled();
  });

  test("cancel at the final confirmation dispatches nothing", () => {
    const onDeactivate = renderSection();
    fireEvent.click(screen.getByTestId("deactivate-account-button"));
    typePassword("synthetic-password");
    fireEvent.click(screen.getByTestId("deactivate-account-continue"));
    fireEvent.click(screen.getByTestId("deactivate-account-confirm-cancel"));

    expect(onDeactivate).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  test("the entry point is hidden when the server does not allow deactivation", () => {
    renderSection(vi.fn(), {
      change_password: { kind: "enabled" },
      deactivate_account: { kind: "disabled" }
    });
    expect(screen.queryByTestId("deactivate-account-button")).toBeNull();
  });

  test("a rejected password returns to the password step with an error", () => {
    const onDeactivate = vi.fn();
    const props = {
      userId: "@member:example.invalid",
      onCancel: vi.fn(),
      onDeactivate
    };
    const { rerender } = render(<DeactivateAccountDialog {...props} accountManagement={idle} />);
    typePassword("wrong-password");
    fireEvent.click(screen.getByTestId("deactivate-account-continue"));
    fireEvent.click(screen.getByTestId("deactivate-account-confirm"));
    rerender(
      <DeactivateAccountDialog
        {...props}
        accountManagement={{ kind: "working", request_id: 4, operation: "deactivateAccount" }}
      />
    );
    expect((screen.getByTestId("deactivate-account-confirm") as HTMLButtonElement).disabled).toBe(true);
    rerender(
      <DeactivateAccountDialog
        {...props}
        accountManagement={{
          kind: "failed",
          request_id: 4,
          operation: "deactivateAccount",
          failureKind: "forbidden"
        }}
      />
    );

    expect(screen.getByRole("alert").textContent).toBe(
      "Incorrect password. The account was not deactivated."
    );
    expect((screen.getByTestId("deactivate-account-continue") as HTMLButtonElement).disabled).toBe(true);
  });
  test("after confirming, the dialog ignores a second click and Escape", () => {
    const onDeactivate = vi.fn();
    const onCancel = vi.fn();
    render(
      <DeactivateAccountDialog
        userId="@member:example.invalid"
        accountManagement={idle}
        onCancel={onCancel}
        onDeactivate={onDeactivate}
      />
    );
    typePassword("synthetic-password");
    fireEvent.click(screen.getByTestId("deactivate-account-continue"));
    const confirm = screen.getByTestId("deactivate-account-confirm");
    fireEvent.click(confirm);
    fireEvent.click(confirm);
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });

    expect(onDeactivate).toHaveBeenCalledOnce();
    expect(onCancel).not.toHaveBeenCalled();
    expect(screen.getByTestId("deactivate-account-final-confirm")).toBeTruthy();
  });
});
