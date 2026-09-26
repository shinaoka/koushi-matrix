// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";

import { t } from "../i18n/messages";
import type {
  RoomJoinRule,
  RoomManagementOperationState,
  RoomManagementState,
  SpaceSummary
} from "../domain/types";
import { SpaceInfoPanel } from "./SpaceInfoPanel";

afterEach(cleanup);

const SPACE_ID = "!space-work:example.invalid";

function space(joinRule: RoomJoinRule | null = null, spaceId = SPACE_ID): SpaceSummary {
  return {
    space_id: spaceId,
    raw_name: "Workspace",
    display_name: "Workspace",
    avatar: null,
    join_rule: joinRule,
    child_room_ids: []
  };
}

function management(
  joinRule: RoomJoinRule,
  {
    canChange = true,
    operation = { kind: "idle" },
    roomId = SPACE_ID
  }: { canChange?: boolean; operation?: RoomManagementOperationState; roomId?: string } = {}
): RoomManagementState {
  return {
    selected_room_id: roomId,
    operation,
    settings: {
      room_id: roomId,
      name: "Workspace",
      topic: null,
      avatar_url: null,
      join_rule: joinRule,
      history_visibility: "shared",
      permissions: {
        can_edit_settings: false,
        can_change_join_rule: canChange,
        can_edit_roles: false,
        can_invite: false,
        can_kick: false,
        can_ban: false,
        can_unban: false
      },
      members: []
    }
  };
}

function access() {
  return screen.getByRole("region", { name: t("space.access") });
}

describe("Space access (#935)", () => {
  test.each([
    ["public", "space.accessPublic"],
    ["invite", "space.accessInvite"],
    ["knock", "space.accessKnock"],
    ["restricted", "space.accessRestricted"],
    ["knockRestricted", "space.accessKnockRestricted"],
    ["private", "space.accessPrivate"],
    ["unknown", "space.accessUnknown"]
  ] as const)("a %s Space shows its own access mode", (rule, message) => {
    render(
      <SpaceInfoPanel
        fallbackName="Workspace"
        rooms={[]}
        space={space(rule)}
        roomManagement={management(rule)}
      />
    );

    expect(within(access()).getByText(t(message))).toBeTruthy();
    if (rule !== "public") {
      expect(within(access()).queryByText(t("space.accessPublic"))).toBeNull();
    }
    if (rule !== "invite") {
      expect(within(access()).queryByText(t("space.accessInvite"))).toBeNull();
    }
  });

  test("an unknown rule is loading, not assumed private", () => {
    render(<SpaceInfoPanel fallbackName="Workspace" rooms={[]} space={space(null)} />);

    expect(within(access()).getByText(t("space.accessLoading"))).toBeTruthy();
    expect(within(access()).queryByText(t("space.accessInvite"))).toBeNull();
    expect(within(access()).queryByRole("button")).toBeNull();
  });

  test("the synced rule shows while permissions are still being checked", () => {
    render(<SpaceInfoPanel fallbackName="Workspace" rooms={[]} space={space("public")} />);

    expect(within(access()).getByText(t("space.accessPublic"))).toBeTruthy();
    expect(within(access()).getByText(t("space.accessCheckingPermission"))).toBeTruthy();
    expect(within(access()).queryByRole("button")).toBeNull();
  });

  test("another Space's settings do not stand in for this one", () => {
    render(
      <SpaceInfoPanel
        fallbackName="Workspace"
        rooms={[]}
        space={space("invite")}
        roomManagement={management("public", { roomId: "!other:example.invalid" })}
      />
    );

    expect(within(access()).getByText(t("space.accessInvite"))).toBeTruthy();
    expect(within(access()).getByText(t("space.accessCheckingPermission"))).toBeTruthy();
  });

  test("a member without the permission sees the mode and why it cannot be changed", () => {
    render(
      <SpaceInfoPanel
        fallbackName="Workspace"
        rooms={[]}
        space={space("invite")}
        roomManagement={management("invite", { canChange: false })}
        onUpdateJoinRule={vi.fn()}
      />
    );

    expect(within(access()).getByText(t("space.accessInvite"))).toBeTruthy();
    expect(within(access()).getByText(t("space.accessNoPermission"))).toBeTruthy();
    expect(within(access()).queryByRole("button")).toBeNull();
  });

  test("making a Space public explains the effect and submits only once confirmed", () => {
    const onUpdateJoinRule = vi.fn();
    render(
      <SpaceInfoPanel
        fallbackName="Workspace"
        rooms={[]}
        space={space("invite")}
        roomManagement={management("invite")}
        onUpdateJoinRule={onUpdateJoinRule}
      />
    );

    expect(within(access()).queryByRole("button", { name: t("space.accessMakePrivate") })).toBeNull();
    fireEvent.click(within(access()).getByRole("button", { name: t("space.accessMakePublic") }));
    expect(onUpdateJoinRule).not.toHaveBeenCalled();
    expect(within(access()).getByText(t("space.accessConfirmPublic"))).toBeTruthy();
    // Focus moves into the confirmation rather than falling to the page.
    let dialog = within(access()).getByRole("group", { name: t("space.accessMakePublic") });
    expect(document.activeElement).toBe(
      within(dialog).getByRole("button", { name: t("space.accessMakePublic") })
    );

    fireEvent.click(within(access()).getByRole("button", { name: t("action.cancel") }));
    expect(within(access()).queryByText(t("space.accessConfirmPublic"))).toBeNull();
    expect(onUpdateJoinRule).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(
      within(access()).getByRole("button", { name: t("space.accessMakePublic") })
    );

    fireEvent.click(within(access()).getByRole("button", { name: t("space.accessMakePublic") }));
    dialog = within(access()).getByRole("group", { name: t("space.accessMakePublic") });
    fireEvent.click(within(dialog).getByRole("button", { name: t("space.accessMakePublic") }));
    expect(onUpdateJoinRule).toHaveBeenCalledTimes(1);
    expect(document.activeElement).toBe(
      within(access()).getByRole("heading", { name: t("space.access") })
    );
    expect(onUpdateJoinRule).toHaveBeenCalledWith(SPACE_ID, "public");
  });

  test("making a Space private sends the invite rule", () => {
    const onUpdateJoinRule = vi.fn();
    render(
      <SpaceInfoPanel
        fallbackName="Workspace"
        rooms={[]}
        space={space("public")}
        roomManagement={management("public")}
        onUpdateJoinRule={onUpdateJoinRule}
      />
    );

    fireEvent.click(within(access()).getByRole("button", { name: t("space.accessMakePrivate") }));
    expect(within(access()).getByText(t("space.accessConfirmPrivate"))).toBeTruthy();
    const dialog = within(access()).getByRole("group", { name: t("space.accessMakePrivate") });
    fireEvent.click(within(dialog).getByRole("button", { name: t("space.accessMakePrivate") }));
    expect(onUpdateJoinRule).toHaveBeenCalledWith(SPACE_ID, "invite");
  });

  test("leaving a rule outside the binary names the rule being replaced", () => {
    render(
      <SpaceInfoPanel
        fallbackName="Workspace"
        rooms={[]}
        space={space("restricted")}
        roomManagement={management("restricted")}
        onUpdateJoinRule={vi.fn()}
      />
    );

    fireEvent.click(within(access()).getByRole("button", { name: t("space.accessMakePrivate") }));
    expect(
      within(access()).getByText(t("space.accessReplacesRule", { rule: t("room.joinRuleRestricted") }))
    ).toBeTruthy();
  });

  test("a pending change disables every access action", () => {
    render(
      <SpaceInfoPanel
        fallbackName="Workspace"
        rooms={[]}
        space={space("invite")}
        roomManagement={management("invite", {
          operation: { kind: "pending", request_id: 1, room_id: SPACE_ID, operation: "settings" }
        })}
        onUpdateJoinRule={vi.fn()}
      />
    );

    expect(within(access()).getByText(t("space.accessSaving"))).toBeTruthy();
    const makePublic = within(access()).getByRole("button", { name: t("space.accessMakePublic") });
    expect((makePublic as HTMLButtonElement).disabled).toBe(true);
  });

  test("a second submission cannot start while the first is still in flight", () => {
    const onUpdateJoinRule = vi.fn(() => new Promise<void>(() => undefined));
    render(
      <SpaceInfoPanel
        fallbackName="Workspace"
        rooms={[]}
        space={space("invite")}
        roomManagement={management("invite")}
        onUpdateJoinRule={onUpdateJoinRule}
      />
    );
    fireEvent.click(within(access()).getByRole("button", { name: t("space.accessMakePublic") }));
    const dialog = within(access()).getByRole("group", { name: t("space.accessMakePublic") });
    fireEvent.click(within(dialog).getByRole("button", { name: t("space.accessMakePublic") }));

    // Rust has not reported Pending yet, but the section already treats it as saving.
    expect(within(access()).getByText(t("space.accessSaving"))).toBeTruthy();
    const again = within(access()).getByRole("button", { name: t("space.accessMakePublic") });
    expect((again as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(again);
    expect(onUpdateJoinRule).toHaveBeenCalledTimes(1);
  });

  test("a rejected change is reported and can be retried; a saved one is confirmed", async () => {
    const onUpdateJoinRule = vi.fn();
    const props = {
      fallbackName: "Workspace",
      rooms: [],
      space: space("invite"),
      onUpdateJoinRule
    };
    const { rerender } = render(
      <SpaceInfoPanel {...props} roomManagement={management("invite")} />
    );
    fireEvent.click(within(access()).getByRole("button", { name: t("space.accessMakePublic") }));
    const dialog = within(access()).getByRole("group", { name: t("space.accessMakePublic") });
    fireEvent.click(within(dialog).getByRole("button", { name: t("space.accessMakePublic") }));
    await act(async () => undefined);

    rerender(
      <SpaceInfoPanel
        {...props}
        roomManagement={management("invite", {
          operation: {
            kind: "failed",
            request_id: 1,
            room_id: SPACE_ID,
            operation: "settings",
            failureKind: "network"
          }
        })}
      />
    );
    expect(within(access()).getByText(t("space.accessFailed"))).toBeTruthy();
    expect(within(access()).getByText(t("space.accessInvite"))).toBeTruthy();
    expect(within(access()).queryByText(t("space.accessSaved"))).toBeNull();

    rerender(
      <SpaceInfoPanel
        {...props}
        roomManagement={management("invite", {
          operation: {
            kind: "failed",
            request_id: 1,
            room_id: SPACE_ID,
            operation: "settings",
            failureKind: "forbidden"
          }
        })}
      />
    );
    expect(within(access()).getByText(t("space.accessForbidden"))).toBeTruthy();

    // Retry.
    fireEvent.click(within(access()).getByRole("button", { name: t("space.accessMakePublic") }));
    const retry = within(access()).getByRole("group", { name: t("space.accessMakePublic") });
    fireEvent.click(within(retry).getByRole("button", { name: t("space.accessMakePublic") }));
    await act(async () => undefined);
    expect(onUpdateJoinRule).toHaveBeenCalledTimes(2);

    rerender(<SpaceInfoPanel {...props} roomManagement={management("public")} />);
    expect(within(access()).getByText(t("space.accessPublic"))).toBeTruthy();
    expect(within(access()).getByText(t("space.accessSaved"))).toBeTruthy();
  });

  test("a failure left over from before this panel's own submission is not shown", () => {
    render(
      <SpaceInfoPanel
        fallbackName="Workspace"
        rooms={[]}
        space={space("invite")}
        roomManagement={management("invite", {
          operation: {
            kind: "failed",
            request_id: 1,
            room_id: SPACE_ID,
            operation: "settings",
            failureKind: "network"
          }
        })}
        onUpdateJoinRule={vi.fn()}
      />
    );

    expect(within(access()).queryByText(t("space.accessFailed"))).toBeNull();
  });

  test("switching Spaces drops an unconfirmed change", () => {
    const onUpdateJoinRule = vi.fn();
    const { rerender } = render(
      <SpaceInfoPanel
        fallbackName="Workspace"
        rooms={[]}
        space={space("invite")}
        roomManagement={management("invite")}
        onUpdateJoinRule={onUpdateJoinRule}
      />
    );
    fireEvent.click(within(access()).getByRole("button", { name: t("space.accessMakePublic") }));

    const otherId = "!space-other:example.invalid";
    rerender(
      <SpaceInfoPanel
        fallbackName="Workspace"
        rooms={[]}
        space={space("invite", otherId)}
        roomManagement={management("invite", { roomId: otherId })}
        onUpdateJoinRule={onUpdateJoinRule}
      />
    );

    expect(within(access()).queryByText(t("space.accessConfirmPublic"))).toBeNull();
    expect(onUpdateJoinRule).not.toHaveBeenCalled();
  });

  test("the All rooms view has no access section", () => {
    render(<SpaceInfoPanel fallbackName="Home" rooms={[]} space={null} />);

    expect(screen.queryByRole("region", { name: t("space.access") })).toBeNull();
  });

  test("the Access entry leads to the access section", () => {
    render(
      <SpaceInfoPanel
        fallbackName="Workspace"
        rooms={[]}
        space={space("invite")}
        roomManagement={management("invite")}
      />
    );

    const entry = screen.getByRole("button", { name: t("space.access") });
    expect((entry as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(entry);
    expect(document.activeElement).toBe(within(access()).getByRole("heading", { name: t("space.access") }));
  });
});
