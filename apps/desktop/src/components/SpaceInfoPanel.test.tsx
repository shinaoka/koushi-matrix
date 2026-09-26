// @vitest-environment jsdom
import { renderToStaticMarkup } from "react-dom/server";
import { act, cleanup, createEvent, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";

import { SpaceInfoPanel } from "./SpaceInfoPanel";
import { t } from "../i18n/messages";
import type { SpaceSummary } from "../domain/types";

afterEach(cleanup);

describe("SpaceInfoPanel", () => {
  test("renders space identity, child rooms, unread total, and Element-like entries", () => {
    const markup = renderToStaticMarkup(
      <SpaceInfoPanel
        fallbackName="Synthetic Workspace"
        rooms={[
          {
            room_id: "!room-alpha:example.invalid",
            display_name: "Alpha Upstream",
            display_label: "Alpha Local",
            original_display_label: "Alpha Upstream",
            avatar: null,
            is_dm: false,
            dm_user_ids: [],
            tags: { favourite: null, low_priority: null },
            parent_space_ids: ["!space-work:example.invalid"],
            dm_space_ids: [],
            is_encrypted: false,
            unread_count: 8
          },
          {
            room_id: "!room-beta:example.invalid",
            display_name: "Beta Room",
            display_label: "Beta Room",
            original_display_label: "Beta Room",
            avatar: null,
            is_dm: false,
            dm_user_ids: [],
            tags: { favourite: null, low_priority: null },
            parent_space_ids: ["!space-work:example.invalid"],
            dm_space_ids: [],
            is_encrypted: false,
            unread_count: 2
          },
          {
            room_id: "!dm-alice:example.invalid",
            display_name: "Alice",
            display_label: "Alice",
            original_display_label: "Alice",
            avatar: null,
            is_dm: true,
            dm_user_ids: ["@alice:example.invalid"],
            tags: { favourite: null, low_priority: null },
            parent_space_ids: ["!space-work:example.invalid"],
            dm_space_ids: [],
            is_encrypted: false,
            unread_count: 4
          }
        ]}
        space={{
          space_id: "!space-work:example.invalid",
          raw_name: null,
          display_name: "Synthetic Workspace",
          avatar: null,
          join_rule: null,
          child_room_ids: ["!room-alpha:example.invalid", "!room-beta:example.invalid"]
        }}
      />
    );

    expect(markup).toContain("Synthetic Workspace");
    expect(markup).toContain("!space-work:example.invalid");
    expect(markup).toContain("Rooms");
    expect(markup).toContain("2");
    expect(markup).toContain("Unread");
    expect(markup).toContain("10");
    expect(markup).toContain("Alpha Local");
    expect(markup).not.toContain("Alpha Upstream");
    expect(markup).toContain("Beta Room");
    expect(markup).not.toContain("Alice");
    expect(markup).toContain("Invite");
    expect(markup).toContain("Space preferences");
    expect(markup).toContain("Room membership");
    expect(markup).toContain("Child rooms");
    expect(markup).toContain("Direct Messages");
    expect(markup).toContain("Global DM list");
  });

  test("renders account home summary when no Space is selected", () => {
    const markup = renderToStaticMarkup(
      <SpaceInfoPanel
        fallbackName="Home"
        rooms={[
          {
            room_id: "!room-alpha:example.invalid",
            display_name: "Alpha Room",
            display_label: "Alpha Room",
            original_display_label: "Alpha Room",
            avatar: null,
            is_dm: false,
            dm_user_ids: [],
            tags: { favourite: null, low_priority: null },
            parent_space_ids: [],
            dm_space_ids: [],
            is_encrypted: false,
            unread_count: 8
          },
          {
            room_id: "!dm-alice:example.invalid",
            display_name: "Alice",
            display_label: "Alice",
            original_display_label: "Alice",
            avatar: null,
            is_dm: true,
            dm_user_ids: ["@alice:example.invalid"],
            tags: { favourite: null, low_priority: null },
            parent_space_ids: [],
            dm_space_ids: [],
            is_encrypted: false,
            unread_count: 4
          }
        ]}
        space={null}
      />
    );

    expect(markup).toContain("Home");
    expect(markup).toContain("All rooms");
    expect(markup).toContain("Alpha Room");
    expect(markup).not.toContain("Alice");
  });

  test("does not render a dense member list in the space info panel", () => {
    render(
      <SpaceInfoPanel
        fallbackName="Synthetic Workspace"
        rooms={[]}
        space={{
          space_id: "!space-work:example.invalid",
          raw_name: null,
          display_name: "Synthetic Workspace",
          avatar: null,
          join_rule: null,
          child_room_ids: []
        }}
        roomManagement={{
          selected_room_id: "!space-work:example.invalid",
          settings: {
            room_id: "!space-work:example.invalid",
            name: "Synthetic Workspace",
            topic: null,
            avatar_url: null,
            join_rule: "invite",
            history_visibility: "shared",
            permissions: {
              can_edit_settings: false,
              can_change_join_rule: false,
              can_edit_roles: false,
              can_invite: false,
              can_kick: false,
              can_ban: false,
              can_unban: false
            },
            members: [
              {
                user_id: "@ada:example.invalid",
                display_name: "Ada",
                display_label: "Ada",
                original_display_label: "Ada",
                avatar_url: null,
                power_level: 0,
                role: "user",
                membership: "joined" as const,
                role_options: []
              }
            ]
          },
          operation: { kind: "idle" }
        }}
      />
    );

    expect(screen.queryByRole("button", { name: "Message Ada" })).toBeNull();
    expect(screen.queryByText("@ada:example.invalid")).toBeNull();
  });

  test("opens the standalone people panel from the members entry", () => {
    const onOpenMembers = vi.fn();
    render(
      <SpaceInfoPanel
        fallbackName="Synthetic Workspace"
        rooms={[]}
        space={{
          space_id: "!space-work:example.invalid",
          raw_name: null,
          display_name: "Synthetic Workspace",
          avatar: null,
          join_rule: null,
          child_room_ids: []
        }}
        onOpenMembers={onOpenMembers}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "Members" }));
    expect(onOpenMembers).toHaveBeenCalledTimes(1);
  });

  // Issue #1008: the entry list leads somewhere or is not there.
  test("offers no dead-end entries and names the access entry after its target", () => {
    render(
      <SpaceInfoPanel
        fallbackName="Synthetic Workspace"
        rooms={[]}
        space={workSpace()}
        onInvitePeople={vi.fn()}
        onOpenFiles={vi.fn()}
        onOpenMembers={vi.fn()}
      />
    );

    for (const label of ["Home", "Preferences", "Space settings", "Notifications"]) {
      expect(screen.queryByRole("button", { name: label })).toBeNull();
    }
    const entries = screen
      .getAllByRole("button")
      .filter((button) => button.classList.contains("settings-list-item"));
    expect(entries.map((entry) => entry.textContent)).toEqual([
      "Access",
      "Members",
      "Invite",
      "Files"
    ]);
    expect(entries.every((entry) => !(entry as HTMLButtonElement).disabled)).toBe(true);
  });

  test("keeps auxiliary history download after the Space's own properties", () => {
    render(
      <SpaceInfoPanel
        fallbackName="Synthetic Workspace"
        rooms={[]}
        space={workSpace()}
        historyExport={{ kind: "idle" } as never}
        historyExportControls={{} as never}
        onSetLocalPresentation={vi.fn()}
      />
    );

    const names = screen.getByRole("region", { name: "Names" });
    const access = screen.getByRole("region", { name: "Access" });
    const rooms = screen.getByRole("region", { name: "Rooms" });
    const download = screen.getByRole("region", { name: t("historyExport.spaceSection") });
    // The access section follows the names directly: nothing splits the
    // Space's own properties, and the download comes after its rooms.
    expect(names.nextElementSibling).toBe(access);
    expect(rooms.compareDocumentPosition(download) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  test("shows and edits the local name in one card, without a second copy elsewhere", () => {
    const onSetLocalPresentation = vi.fn();
    render(
      <SpaceInfoPanel
        fallbackName="Synthetic Workspace"
        localIcon="SW"
        localName="Research"
        rooms={[]}
        space={workSpace()}
        onSetLocalPresentation={onSetLocalPresentation}
      />
    );

    // One place: the label appears once, no textbox is open, and nothing
    // saves until asked.
    expect(screen.getAllByText("Local name")).toHaveLength(1);
    expect(screen.queryByRole("region", { name: "Local presentation" })).toBeNull();
    expect(screen.queryByRole("textbox")).toBeNull();
    const card = localNameCard();
    expect(card.textContent).toContain("Research");

    fireEvent.click(within(card).getByRole("button", { name: "Edit local name" }));
    const field = within(card).getByRole("textbox", { name: "Local name" }) as HTMLInputElement;
    expect(document.activeElement).toBe(field);
    expect(field.value).toBe("Research");
    fireEvent.change(field, { target: { value: "  Lab  " } });
    expect(onSetLocalPresentation).not.toHaveBeenCalled();
    fireEvent.click(within(card).getByRole("button", { name: "Save local name" }));

    expect(onSetLocalPresentation).toHaveBeenCalledTimes(1);
    expect(onSetLocalPresentation).toHaveBeenCalledWith({ name: "Lab", icon: "SW" });
    expect(within(localNameCard()).queryByRole("textbox")).toBeNull();
    expect(document.activeElement).toBe(within(localNameCard()).getByRole("heading", { name: "Local name" }));
  });

  test("cancel and Escape close the editor without saving and return focus to Edit", () => {
    const onSetLocalPresentation = vi.fn();
    render(
      <SpaceInfoPanel
        fallbackName="Synthetic Workspace"
        localName="Research"
        rooms={[]}
        space={workSpace()}
        onSetLocalPresentation={onSetLocalPresentation}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "Edit local name" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Local name" }), { target: { value: "Draft" } });
    fireEvent.click(within(localNameCard()).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("textbox")).toBeNull();
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Edit local name" }));

    fireEvent.click(screen.getByRole("button", { name: "Edit local name" }));
    const field = screen.getByRole("textbox", { name: "Local name" });
    expect((field as HTMLInputElement).value).toBe("Research");
    fireEvent.keyDown(field, { key: "Escape" });
    expect(screen.queryByRole("textbox")).toBeNull();
    expect(onSetLocalPresentation).not.toHaveBeenCalled();
  });

  test("an IME confirmation Enter does not save the local name", () => {
    vi.useFakeTimers();
    try {
      const onSetLocalPresentation = vi.fn();
      render(
        <SpaceInfoPanel
          fallbackName="Synthetic Workspace"
          rooms={[]}
          space={workSpace()}
          onSetLocalPresentation={onSetLocalPresentation}
        />
      );

      fireEvent.click(screen.getByRole("button", { name: "Edit local name" }));
      const field = screen.getByRole("textbox", { name: "Local name" });
      const form = field.closest("form") as HTMLFormElement;
      fireEvent.compositionStart(field);
      fireEvent.change(field, { target: { value: "研究" } });
      const imeEnter = createEvent.keyDown(field, {
        key: "Enter",
        code: "Enter",
        keyCode: 229,
        isComposing: true
      });
      fireEvent(field, imeEnter);
      fireEvent.submit(form);

      expect(imeEnter.defaultPrevented).toBe(false);
      expect(onSetLocalPresentation).not.toHaveBeenCalled();
      expect(screen.getByRole("textbox", { name: "Local name" })).toBeTruthy();

      fireEvent.compositionEnd(field);
      act(() => {
        vi.runAllTimers();
      });
      fireEvent.submit(form);
      expect(onSetLocalPresentation).toHaveBeenCalledWith({ name: "研究", icon: null });
    } finally {
      vi.useRealTimers();
    }
  });

  test("clearing the local name keeps the local icon, and the reverse", async () => {
    const onSetLocalPresentation = vi.fn();
    render(
      <SpaceInfoPanel
        fallbackName="Synthetic Workspace"
        localIcon="SW"
        localName="Research"
        rooms={[]}
        space={workSpace()}
        onSetLocalPresentation={onSetLocalPresentation}
      />
    );

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Clear local name" }));
    });
    expect(onSetLocalPresentation).toHaveBeenLastCalledWith({ name: null, icon: "SW" });
    fireEvent.click(screen.getByRole("button", { name: "Clear local icon" }));
    expect(onSetLocalPresentation).toHaveBeenLastCalledWith({ name: "Research", icon: null });
  });

  test("clearing the last local field removes the Space's local presentation", () => {
    const onSetLocalPresentation = vi.fn();
    render(
      <SpaceInfoPanel
        fallbackName="Synthetic Workspace"
        localIcon="SW"
        rooms={[]}
        space={workSpace()}
        onSetLocalPresentation={onSetLocalPresentation}
      />
    );

    // Nothing to clear on an unset name: the card offers setting it instead.
    expect(screen.queryByRole("button", { name: "Clear local name" })).toBeNull();
    expect(localNameCard().textContent).toContain("Not set");
    expect(within(localNameCard()).getByRole("button", { name: "Edit local name" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Clear local icon" }));
    expect(onSetLocalPresentation).toHaveBeenLastCalledWith(null);
  });

  test("reports saving, then saved only once Rust's value is the submitted one", async () => {
    let admit!: () => void;
    const onSetLocalPresentation = vi.fn(
      () => new Promise<void>((resolve) => { admit = resolve; })
    );
    const view = (localName: string) => (
      <SpaceInfoPanel
        fallbackName="Synthetic Workspace"
        localName={localName}
        rooms={[]}
        space={workSpace()}
        onSetLocalPresentation={onSetLocalPresentation}
      />
    );
    const { rerender } = render(view("Research"));

    fireEvent.click(screen.getByRole("button", { name: "Edit local name" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Local name" }), { target: { value: "Lab" } });
    fireEvent.click(screen.getByRole("button", { name: "Save local name" }));

    expect(within(localNameCard()).getByRole("status").textContent).toBe("Saving…");
    expect(within(localNameCard()).getByRole("button", { name: "Edit local name" })).toHaveProperty("disabled", true);
    // The icon card is not the one being saved.
    expect(within(localIconCard()).queryByRole("status")).toBeNull();

    await act(async () => admit());
    // Admitted, but the confirmed value is still the old one: no success.
    expect(within(localNameCard()).queryByRole("status")).toBeNull();
    expect(localNameCard().textContent).toContain("Research");

    rerender(view("Lab"));
    expect(within(localNameCard()).getByRole("status").textContent).toBe("Saved");
    expect(localNameCard().textContent).toContain("Lab");
  });

  test("a rejected save is reported in the card and can be retried", async () => {
    const onSetLocalPresentation = vi
      .fn()
      .mockRejectedValueOnce(new Error("synthetic rejection"))
      .mockResolvedValueOnce(undefined);
    render(
      <SpaceInfoPanel
        fallbackName="Synthetic Workspace"
        localName="Research"
        rooms={[]}
        space={workSpace()}
        onSetLocalPresentation={onSetLocalPresentation}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "Edit local name" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Local name" }), { target: { value: "Lab" } });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Save local name" }));
    });

    expect(within(localNameCard()).getByRole("status").textContent).toBe(
      "Could not save this on this device. Try again."
    );
    expect(localNameCard().textContent).toContain("Research");

    fireEvent.click(screen.getByRole("button", { name: "Edit local name" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Local name" }), { target: { value: "Lab" } });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Save local name" }));
    });
    expect(onSetLocalPresentation).toHaveBeenCalledTimes(2);
    expect(within(localNameCard()).queryByRole("status")).toBeNull();
  });

  test("switching Spaces drops an open editor and a pending result", async () => {
    let admit!: () => void;
    const onSetLocalPresentation = vi.fn(
      () => new Promise<void>((resolve) => { admit = resolve; })
    );
    const view = (space: SpaceSummary, localName: string) => (
      <SpaceInfoPanel
        fallbackName="Synthetic Workspace"
        localName={localName}
        rooms={[]}
        space={space}
        onSetLocalPresentation={onSetLocalPresentation}
      />
    );
    const { rerender } = render(view(workSpace(), "Research"));

    fireEvent.click(screen.getByRole("button", { name: "Edit local name" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Local name" }), { target: { value: "Lab" } });
    fireEvent.click(screen.getByRole("button", { name: "Save local name" }));
    fireEvent.click(screen.getByRole("button", { name: "Edit local icon" }));

    const other = workSpace("!space-other:example.invalid");
    rerender(view(other, "Lab"));
    await act(async () => admit());

    expect(screen.queryByRole("textbox")).toBeNull();
    expect(within(localNameCard()).queryByRole("status")).toBeNull();
    expect(onSetLocalPresentation).toHaveBeenCalledTimes(1);
  });

  test("without a save handler the local name is shown read-only in the same card", () => {
    render(
      <SpaceInfoPanel
        fallbackName="Synthetic Workspace"
        localName="Research"
        rooms={[]}
        space={workSpace()}
      />
    );

    expect(localNameCard().textContent).toContain("Research");
    expect(within(localNameCard()).queryByRole("button")).toBeNull();
  });

  // Issue #960: a local presentation name must not hide what the Space is
  // called on Matrix, and a Space without an `m.room.name` must not have an
  // alias or a computed name presented as its canonical one.
  test("shows the canonical Matrix name and the local name as separate facts", () => {
    render(
      <SpaceInfoPanel
        fallbackName="Fallback"
        localName="My Shortcut"
        rooms={[]}
        space={{
          space_id: "!space-work:example.invalid",
          raw_name: "Research Group",
          display_name: "Research Group",
          avatar: null,
          join_rule: null,
          child_room_ids: []
        }}
      />
    );

    const canonical = screen.getByText("Matrix name").closest("[data-setting-property]");
    expect(canonical?.textContent).toContain("Research Group");
    expect(canonical?.textContent).not.toContain("My Shortcut");
    const local = localNameCard();
    expect(local.textContent).toContain("My Shortcut");
    expect(local.textContent).not.toContain("Research Group");
    // The local name still wins the panel title, as it did before.
    expect(screen.getByRole("heading", { name: "My Shortcut" })).toBeTruthy();
  });

  test("reports a Space with no m.room.name as unnamed rather than borrowing its computed name", () => {
    render(
      <SpaceInfoPanel
        fallbackName="Fallback"
        rooms={[]}
        space={{
          space_id: "!space-work:example.invalid",
          raw_name: null,
          display_name: "Alice and Bob",
          avatar: null,
          join_rule: null,
          child_room_ids: []
        }}
      />
    );

    const canonical = screen.getByText("Matrix name").closest("[data-setting-property]");
    expect(canonical?.textContent).toContain("Not set");
    expect(canonical?.textContent).not.toContain("Alice and Bob");
  });

  // Issue #961: the Space's own room list shows the whole Space, not only the
  // rooms this account happens to have joined.
  test("lists children the account is not in, with their membership and a join action", () => {
    const onJoinRoom = vi.fn();
    render(
      <SpaceInfoPanel
        fallbackName="Fallback"
        rooms={[]}
        space={{
          space_id: "!space-work:example.invalid",
          raw_name: "Work",
          display_name: "Work",
          avatar: null,
          join_rule: null,
          child_room_ids: []
        }}
        spaceChildren={[
          {
            room_id: "!open:example.invalid",
            display_name: "Open Room",
            avatar: null,
            membership: "not_joined",
            can_join: true,
            is_space: false,
            joined_members: 4
          },
          {
            room_id: "!invited:example.invalid",
            display_name: "Invited Room",
            avatar: null,
            membership: "invited",
            can_join: true,
            is_space: false,
            joined_members: 2
          },
          {
            room_id: "!private:example.invalid",
            display_name: "!private:example.invalid",
            avatar: null,
            membership: "unknown",
            can_join: false,
            is_space: false,
            joined_members: 0
          }
        ]}
        onJoinRoom={onJoinRoom}
      />
    );

    const open = screen.getByText("Open Room").closest(".settings-detail-row");
    expect(open?.textContent).toContain("Not joined");
    const invited = screen.getByText("Invited Room").closest(".settings-detail-row");
    expect(invited?.textContent).toContain("Invited");

    // A room whose details the server withheld offers no join: the server's
    // permission model decides, and the panel does not guess around it.
    const unavailable = screen
      .getByText("!private:example.invalid")
      .closest(".settings-detail-row");
    expect(unavailable?.textContent).toContain("Unavailable");
    expect(unavailable?.querySelector("button")).toBeNull();

    fireEvent.click(open?.querySelector("button") as HTMLButtonElement);
    expect(onJoinRoom).toHaveBeenCalledWith("!open:example.invalid");
  });

  // An invitation belongs to the invite workflow, which owns the account's
  // invite list; joining around it would leave that list stale.
  test("answers an invited child through the invite workflow, not a join", () => {
    const onAcceptInvite = vi.fn();
    const onJoinRoom = vi.fn();
    render(
      <SpaceInfoPanel
        fallbackName="Fallback"
        rooms={[]}
        space={{
          space_id: "!space-work:example.invalid",
          raw_name: "Work",
          display_name: "Work",
          avatar: null,
          join_rule: null,
          child_room_ids: []
        }}
        spaceChildren={[
          {
            room_id: "!invited:example.invalid",
            display_name: "Invited Room",
            avatar: null,
            membership: "invited",
            can_join: true,
            is_space: false,
            joined_members: 2
          }
        ]}
        onAcceptInvite={onAcceptInvite}
        onJoinRoom={onJoinRoom}
      />
    );

    const invited = screen.getByText("Invited Room").closest(".settings-detail-row");
    fireEvent.click(invited?.querySelector("button") as HTMLButtonElement);
    expect(onAcceptInvite).toHaveBeenCalledWith("!invited:example.invalid");
    expect(onJoinRoom).not.toHaveBeenCalled();
  });

  test("never repeats a joined room in the not-joined part of the list", () => {
    render(
      <SpaceInfoPanel
        fallbackName="Fallback"
        rooms={[
          {
            room_id: "!joined:example.invalid",
            display_name: "Joined Room",
            display_label: "Joined Room",
            original_display_label: "Joined Room",
            avatar: null,
            is_dm: false,
            dm_user_ids: [],
            tags: { favourite: null, low_priority: null },
            parent_space_ids: ["!space-work:example.invalid"],
            dm_space_ids: [],
            is_encrypted: false,
            unread_count: 0
          }
        ]}
        space={{
          space_id: "!space-work:example.invalid",
          raw_name: "Work",
          display_name: "Work",
          avatar: null,
          join_rule: null,
          child_room_ids: ["!joined:example.invalid"]
        }}
        spaceChildren={[
          {
            room_id: "!joined:example.invalid",
            display_name: "Joined Room",
            avatar: null,
            membership: "joined",
            can_join: false,
            is_space: false,
            joined_members: 3
          }
        ]}
      />
    );

    expect(screen.getAllByText("Joined Room")).toHaveLength(1);
  });
});

function workSpace(spaceId = "!space-work:example.invalid"): SpaceSummary {
  return {
    space_id: spaceId,
    raw_name: null,
    display_name: "Synthetic Workspace",
    avatar: null,
    join_rule: null,
    child_room_ids: []
  };
}

function localNameCard(): HTMLElement {
  return document.querySelector('[data-setting-property="space-local-name"]') as HTMLElement;
}

function localIconCard(): HTMLElement {
  return document.querySelector('[data-setting-property="space-local-icon"]') as HTMLElement;
}
