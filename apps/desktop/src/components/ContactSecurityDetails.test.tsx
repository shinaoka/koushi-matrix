// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";

import { ProfilePanel } from "./PeoplePanel";
import type {
  ContactSecurityState,
  ContactSecuritySummary,
  RoomManagementState,
  VerificationFlowState
} from "../domain/types";

const CONTACT = "@ada:example.invalid";
const OTHER = "@grace:example.invalid";
const ME = "@current:example.invalid";

const roomManagement: RoomManagementState = {
  selected_room_id: null,
  settings: null,
  operation: { kind: "idle" }
};

function summary(partial: Partial<ContactSecuritySummary>): ContactSecuritySummary {
  return {
    devices: "allOwnerSigned",
    device_counts: {
      total: 2,
      owner_signed: 2,
      not_owner_signed: 0,
      owner_signature_invalid: 0,
      excluded_dehydrated: 0
    },
    device_signatures: ["ownerSigned", "ownerSigned"],
    identity: "notVerifiedByYou",
    verification: { kind: "notOffered" },
    ...partial
  };
}

const unsignedDeviceVerified = summary({
  devices: "someNotOwnerSigned",
  device_counts: {
    total: 3,
    owner_signed: 1,
    not_owner_signed: 2,
    owner_signature_invalid: 1,
    excluded_dehydrated: 1
  },
  device_signatures: ["ownerSigned", "notOwnerSigned", "ownerSignatureInvalid"],
  identity: "verifiedByYou"
});

function loaded(
  value: ContactSecuritySummary,
  userId = CONTACT,
  verificationBusy = false
): ContactSecurityState {
  return {
    user_id: userId,
    load: { kind: "loaded", request_id: 4 },
    summary: value,
    verification_busy: verificationBusy
  };
}

function mockActions() {
  return {
    load: vi.fn(),
    close: vi.fn(),
    requestVerification: vi.fn(),
    acceptVerification: vi.fn(),
    confirmSas: vi.fn(),
    mismatchSas: vi.fn(),
    cancelVerification: vi.fn()
  };
}

function renderProfile(
  state: ContactSecurityState,
  userId = CONTACT,
  verification: VerificationFlowState = { kind: "idle" }
) {
  const actions = mockActions();
  const view = render(
    <ProfilePanel
      userId={userId}
      currentUserId={ME}
      roomOrSpace={null}
      roomManagement={roomManagement}
      profileUsers={{}}
      contactSecurity={state}
      verification={verification}
      contactSecurityActions={actions}
      onBack={() => undefined}
    />
  );
  return { actions, ...view };
}

function row(label: string): HTMLElement {
  const element = screen.getByText(label).closest(".profile-security-row");
  if (!(element instanceof HTMLElement)) throw new Error(`missing row ${label}`);
  return element;
}

afterEach(cleanup);

describe("ContactSecurityDetails", () => {
  test("opening User info dispatches only the read-only load; closing dispatches close", () => {
    const { actions, rerender, unmount } = renderProfile({
      user_id: null,
      load: { kind: "idle" },
      summary: null,
      verification_busy: false
    });
    expect(actions.load).toHaveBeenCalledTimes(1);
    expect(actions.load).toHaveBeenCalledWith(CONTACT);
    expect(actions.close).not.toHaveBeenCalled();
    expect(within(row("Their devices")).getByText("Checking…")).toBeTruthy();

    // Switching contact closes the previous one and loads the new one.
    rerender(
      <ProfilePanel
        userId={OTHER}
        currentUserId={ME}
        roomOrSpace={null}
        roomManagement={roomManagement}
        profileUsers={{}}
        contactSecurity={loaded(unsignedDeviceVerified)}
        contactSecurityActions={actions}
        onBack={() => undefined}
      />
    );
    expect(actions.close).toHaveBeenCalledTimes(1);
    expect(actions.load).toHaveBeenLastCalledWith(OTHER);
    // The snapshot still belongs to the previous contact: not shown as Grace's.
    expect(within(row("Their devices")).getByText("Checking…")).toBeTruthy();
    expect(screen.queryByText("Verified by you")).toBeNull();

    unmount();
    expect(actions.close).toHaveBeenCalledTimes(2);
  });

  test("keeps the contact's devices and your verification as separate facts", () => {
    const cases: Array<[ContactSecuritySummary, string, string]> = [
      [summary({}), "All confirmed by their owner", "Not verified by you"],
      [summary({ identity: "verifiedByYou" }), "All confirmed by their owner", "Verified by you"],
      [unsignedDeviceVerified, "Some not yet confirmed", "Verified by you"]
    ];
    for (const [value, devices, identity] of cases) {
      renderProfile(loaded(value));
      expect(within(row("Their devices")).getByText(devices)).toBeTruthy();
      expect(within(row("Your verification")).getByText(identity)).toBeTruthy();
      cleanup();
    }
  });

  test("routine unconfirmed devices and never-verified contacts stay neutral", () => {
    renderProfile(
      loaded(
        summary({
          devices: "someNotOwnerSigned",
          device_counts: {
            total: 2,
            owner_signed: 1,
            not_owner_signed: 1,
            owner_signature_invalid: 0,
            excluded_dehydrated: 0
          },
          device_signatures: ["ownerSigned", "notOwnerSigned"]
        })
      )
    );
    expect(document.querySelector(".is-attention")).toBeNull();
    expect(document.querySelector("[role='alert']")).toBeNull();
    expect(document.querySelector("[class*='danger']")).toBeNull();
    expect(screen.queryByText(/Verify this person/)).toBeNull();
  });

  test("a lapsed verification is a distinct attention state that blames neither side", () => {
    renderProfile(loaded(summary({ identity: "changedAfterVerification" })));
    const identity = row("Your verification");
    expect(identity.classList.contains("is-attention")).toBe(true);
    expect(within(identity).getByText("Your verification no longer applies")).toBeTruthy();
    fireEvent.click(within(identity).getByRole("button", { name: /Details/ }));
    // The SDK reports this when either identity was reset after you verified.
    expect(
      within(identity).getByText(
        "You verified this person before, but their identity or yours has been reset since then, so that verification no longer applies. Choose Verify again to compare emoji with them."
      )
    ).toBeTruthy();
    expect(row("Their devices").classList.contains("is-attention")).toBe(false);
  });

  test("expanded details explain owner confirmation with counts and ordinal devices", () => {
    const { actions } = renderProfile(loaded(unsignedDeviceVerified));
    const devices = row("Their devices");
    const toggle = within(devices).getByRole("button", { name: "Details: Their devices" });
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    fireEvent.click(toggle);
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    expect(
      within(devices).getByText(
        "Some of this person's devices have not been confirmed yet. If you are concerned, ask them to confirm their devices in their app."
      )
    ).toBeTruthy();
    expect(within(devices).getByText("1 of 3 devices confirmed by their owner")).toBeTruthy();
    expect(
      within(devices).getByText("Device signatures that don't match this person's current identity: 1")
    ).toBeTruthy();
    expect(within(devices).getByText("Offline recovery devices not counted: 1")).toBeTruthy();
    expect(within(devices).getByText("Device 3")).toBeTruthy();
    expect(within(devices).getByText("Signature doesn't match")).toBeTruthy();
    // Opening or closing an explanation never dispatches anything.
    fireEvent.click(toggle);
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    expect(actions.load).toHaveBeenCalledTimes(1);
    expect(actions.close).not.toHaveBeenCalled();

    const identityToggle = within(row("Your verification")).getByRole("button", {
      name: "Details: Your verification"
    });
    fireEvent.click(identityToggle);
    expect(
      screen.getByText(
        "You verified that this account belongs to the person you know. This doesn't confirm devices they haven't confirmed themselves."
      )
    ).toBeTruthy();
    expect(actions.load).toHaveBeenCalledTimes(1);
  });

  test("retrieval failure is shown as unavailable, never as confirmation, and can be retried", () => {
    const { actions } = renderProfile({
      user_id: CONTACT,
      load: { kind: "failed", request_id: 2, failureKind: "network" },
      summary: null,
      verification_busy: false
    });
    expect(within(row("Their devices")).getByText("Status unavailable")).toBeTruthy();
    expect(within(row("Your verification")).getByText("Status unavailable")).toBeTruthy();
    expect(screen.queryByText(/confirmed by their owner/)).toBeNull();
    // Retry sits next to the status, not behind the collapsed details.
    fireEvent.click(within(row("Their devices")).getByRole("button", { name: "Retry" }));
    expect(actions.load).toHaveBeenCalledTimes(2);
    expect(actions.load).toHaveBeenLastCalledWith(CONTACT);
  });

  test("missing cross-signing and empty device lists are not confirmation", () => {
    renderProfile(
      loaded(
        summary({
          devices: "ownerIdentityMissing",
          device_counts: {
            total: 1,
            owner_signed: 0,
            not_owner_signed: 1,
            owner_signature_invalid: 0,
            excluded_dehydrated: 0
          },
          device_signatures: ["ownerIdentityMissing"],
          identity: "unknown"
        })
      )
    );
    expect(within(row("Their devices")).getByText("Can't be confirmed yet")).toBeTruthy();
    expect(within(row("Your verification")).getByText("Unknown")).toBeTruthy();
    cleanup();

    renderProfile(
      loaded(
        summary({
          devices: "noDevices",
          device_counts: {
            total: 0,
            owner_signed: 0,
            not_owner_signed: 0,
            owner_signature_invalid: 0,
            excluded_dehydrated: 0
          },
          device_signatures: []
        })
      )
    );
    expect(within(row("Their devices")).getByText("No devices found")).toBeTruthy();
    expect(screen.queryByText(/All confirmed/)).toBeNull();
  });

  test("says the details are about keys, not whether the conversation is encrypted", () => {
    renderProfile(loaded(summary({})));
    expect(
      screen.getByText(
        "These details are about this person's keys. They don't show whether a conversation is encrypted."
      )
    ).toBeTruthy();
  });

  test("your own User info shows no contact security details and dispatches nothing", () => {
    const { actions } = renderProfile(loaded(summary({}), ME), ME);
    expect(screen.queryByText("Security")).toBeNull();
    expect(actions.load).not.toHaveBeenCalled();
  });
});

const offered = (direct_chat: "existingEncrypted" | "existingUnencrypted" | "new") =>
  ({ kind: "offered", direct_chat }) as const;

const target = { user_id: CONTACT, device_id: "" };
const emojis = Array.from({ length: 7 }, (_, index) => ({
  symbol: ["🐶", "🐱", "🦁", "🐎", "🦄", "🐷", "🐘"][index]!,
  description: `emoji ${index}`
}));

describe("Verify user", () => {
  test("is an optional action that sends nothing until the confirmation step", () => {
    const { actions } = renderProfile(loaded(summary({ verification: offered("new") })));
    const verify = screen.getByRole("button", { name: "Verify user" });
    expect(verify.className).toContain("profile-text-button");
    // Opening explanations never starts verification.
    fireEvent.click(within(row("Your verification")).getByRole("button", { name: /Details/ }));
    expect(actions.requestVerification).not.toHaveBeenCalled();

    fireEvent.click(verify);
    expect(
      screen.getByText(
        "You don't have a direct chat with this person yet. Koushi creates a new encrypted direct chat with them and waits up to a minute for them to join before sending the request."
      )
    ).toBeTruthy();
    expect(actions.requestVerification).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(actions.requestVerification).not.toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: "Send request" })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Verify user" }));
    fireEvent.click(screen.getByRole("button", { name: "Send request" }));
    expect(actions.requestVerification).toHaveBeenCalledTimes(1);
    expect(actions.requestVerification).toHaveBeenCalledWith(CONTACT);
  });

  test("names the existing direct chat it will use", () => {
    renderProfile(loaded(summary({ verification: offered("existingEncrypted") })));
    fireEvent.click(screen.getByRole("button", { name: "Verify user" }));
    expect(
      screen.getByText(
        "Koushi sends the verification request in your encrypted direct chat with this person. If they have left that chat, Koushi invites them back and waits up to a minute for them to rejoin before sending the request."
      )
    ).toBeTruthy();
  });

  test("is offered only when Rust offers it", () => {
    renderProfile(loaded(summary({ identity: "verifiedByYou" })));
    expect(screen.queryByRole("button", { name: /Verify/ })).toBeNull();
    cleanup();

    renderProfile(loaded(summary({ verification: { kind: "requiresYourCrossSigning" } })));
    expect(screen.queryByRole("button", { name: /Verify/ })).toBeNull();
    expect(screen.getByText(/this session needs your own cross-signing keys/)).toBeTruthy();
    cleanup();

    renderProfile(
      loaded(
        summary({ identity: "changedAfterVerification", verification: offered("existingEncrypted") })
      )
    );
    expect(screen.getByRole("button", { name: "Verify again" })).toBeTruthy();
  });

  test("our own request waits for them instead of offering Accept, and can be cancelled", () => {
    const { actions } = renderProfile(
      loaded(summary({ verification: offered("new") })),
      CONTACT,
      { kind: "requested", request_id: 17, target, initiator: "us" }
    );
    expect(screen.getByText("Waiting for them to accept the request in their app…")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Accept" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Verify user" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(actions.cancelVerification).toHaveBeenCalledWith(17);
  });

  test("a request from them can be accepted", () => {
    const { actions } = renderProfile(loaded(summary({})), CONTACT, {
      kind: "requested",
      request_id: 18,
      target,
      initiator: "them"
    });
    fireEvent.click(screen.getByRole("button", { name: "Accept" }));
    expect(actions.acceptVerification).toHaveBeenCalledWith(18);
  });

  test("emoji comparison lives in the Your verification area", () => {
    const { actions } = renderProfile(loaded(summary({ verification: offered("new") })), CONTACT, {
      kind: "sasPresented",
      request_id: 19,
      target,
      emojis
    });
    const list = screen.getByRole("list", { name: "Emoji to compare" });
    expect(within(list).getAllByRole("listitem")).toHaveLength(7);
    fireEvent.click(screen.getByRole("button", { name: "They match" }));
    expect(actions.confirmSas).toHaveBeenCalledWith(19);
    fireEvent.click(screen.getByRole("button", { name: "They don't match" }));
    expect(actions.mismatchSas).toHaveBeenCalledWith(19);
  });

  test("failure is explained and verification can be tried again", () => {
    renderProfile(loaded(summary({ verification: offered("new") })), CONTACT, {
      kind: "failed",
      request_id: 20,
      target,
      failureKind: "mismatch"
    });
    expect(screen.getByText("The emoji didn't match. Nothing was verified.")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Verify user" })).toBeTruthy();
  });

  test("completion shows the Rust-refreshed identity row", () => {
    renderProfile(loaded(summary({ identity: "verifiedByYou" })), CONTACT, {
      kind: "done",
      request_id: 21,
      target
    });
    expect(screen.getByText("Verification complete.")).toBeTruthy();
    expect(within(row("Your verification")).getByText("Verified by you")).toBeTruthy();
  });

  test("a verification with someone else is not shown here, and Rust marks Verify user busy", () => {
    renderProfile(loaded(summary({ verification: offered("new") }), CONTACT, true), CONTACT, {
      kind: "sasPresented",
      request_id: 22,
      target: { user_id: OTHER, device_id: "" },
      emojis
    });
    expect(screen.queryByRole("list", { name: "Emoji to compare" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Verify user" })).toBeNull();
    expect(
      screen.getByText(
        "Another verification is in progress. You can verify this person after it finishes."
      )
    ).toBeTruthy();
  });
});
