// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";

import { RoomInfoPanel } from "./RoomInfoPanel";
import { setActiveLocaleProfile, t } from "../i18n/messages";
import type {
  LinkPreviewSettingsState,
  RoomAccessPreview,
  RoomHistoryVisibility,
  RoomJoinRule,
  RoomManagementOperationState,
  RoomManagementState,
  RoomNotificationSettings,
  RoomSettingsSnapshot,
  RoomSummary,
  SettingsState,
  SpaceSummary
} from "../domain/types";

const originalClipboard = Object.getOwnPropertyDescriptor(navigator, "clipboard");

const baseRoom: RoomSummary = {
  room_id: "!room-alpha:example.invalid",
  display_name: "Alpha Room",
  display_label: "Alpha Room",
  original_display_label: "Alpha Room",
  avatar: null,
  is_dm: false,
  dm_user_ids: [],
  tags: { favourite: null, low_priority: null },
  parent_space_ids: ["!space-work:example.invalid"],
  dm_space_ids: [],
  is_encrypted: false,
  unread_count: 8
};

const idleSettings: RoomNotificationSettings = {
  mode: { kind: "all" },
  operation: { kind: "idle" }
};

const pendingSettings: RoomNotificationSettings = {
  mode: { kind: "mute" },
  operation: { kind: "pending", request_id: 1 }
};

const baseAppSettings: SettingsState = {
  values: {
    locale: { language_tag: null, text_direction: "auto" },
    appearance: { theme: "dark", density: "comfortable" },
    typography: { font: "system", emoji: "system" },
    keyboard: { composer_send_shortcut: "enter" },
    composer: { math_mode: true, recent_emojis: [] },
    notifications: {
      desktop_notifications: true,
      sound: true,
      badges: true,
      message_previews: true,
      send_read_receipts: true,
      send_typing_notifications: true
    },
    display: {
      code_block_wrap: true,
      hide_redacted: false,
      url_previews_enabled: true,
      encrypted_url_previews_enabled: false
    },
    window: { close_to_tray: true },
            updates: { auto_check: true, include_prereleases: false },
    media: {
      image_upload_compression_policy: {
        threshold_bytes: 1048576,
        threshold_long_edge: 2560,
        target_long_edge: 2048,
        quality_percent: 82
      }
    },
    timeline: {
      auto_load_older_messages: true,
      thread_root_order: { kind: "rootEvent" }
    },
    search_crawler: {
      speed: "standard" as const,
      include_media_captions: true,
      include_filenames: true
    },
    thread_list_order: { kind: "latestReply" },
    room_list_sort: { kind: "activity" },
    sidebar: {
      category: "rooms",
      collapsed: { favourites: false, low_priority: false, not_joined: false }
    },
    legacy_frontend_preferences_imported: false
  },
  persistence: { kind: "idle" }
};

const baseLinkPreviewSettings: LinkPreviewSettingsState = {
  room_overrides: {}
};

afterEach(() => {
  cleanup();
  setActiveLocaleProfile("en", "none");
  vi.restoreAllMocks();
  if (originalClipboard) Object.defineProperty(navigator, "clipboard", originalClipboard);
  else Reflect.deleteProperty(navigator, "clipboard");
});

describe("RoomInfoPanel", () => {
  test("saves the room name from the panel header", () => {
    const onUpdateRoomSetting = vi.fn();
    render(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        roomManagement={{
          selected_room_id: baseRoom.room_id,
          settings: {
            room_id: baseRoom.room_id,
            name: "Alpha Room",
            topic: null,
            avatar_url: null,
            join_rule: "invite",
            history_visibility: "shared",
            permissions: {
              can_edit_settings: true,
              can_change_join_rule: true,
              can_edit_roles: true,
              can_invite: true,
              can_kick: true,
              can_ban: true,
              can_unban: true
            },
            members: []
          },
          operation: { kind: "idle" }
        }}
        onUpdateRoomSetting={onUpdateRoomSetting}
      />
    );

    const name = screen.getByRole("textbox", { name: "Room name" });
    expect(name.getAttribute("dir")).toBe("auto");
    fireEvent.change(name, { target: { value: "Beta Room" } });
    expect(screen.getByRole("button", { name: "Save room name" }).textContent).toBe("Save");
    fireEvent.click(screen.getByRole("button", { name: "Save room name" }));

    expect(onUpdateRoomSetting).toHaveBeenCalledWith(baseRoom.room_id, {
      name: "Beta Room"
    });
    expect(screen.getByRole("textbox", { name: "Room name" }).compareDocumentPosition(
      screen.getByRole("region", { name: "Details" })
    ) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  test("saves join rule and history visibility independently", () => {
    const onUpdateRoomSetting = vi.fn();
    const onSetAccessDraft = vi.fn();
    const scope = { kind: "room" as const, roomId: baseRoom.room_id };
    const view = (rule: RoomJoinRule | null, history: RoomHistoryVisibility | null) => (
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        roomManagement={{
          selected_room_id: baseRoom.room_id,
          settings: roomSettings(),
          draft: { scope, revision: 2, rule, history },
          operation: { kind: "idle" }
        }}
        accessPreview={accessPreviewFixture("access", false)}
        historyPreview={accessPreviewFixture("history", false)}
        onUpdateRoomSetting={onUpdateRoomSetting}
        onSetAccessDraft={onSetAccessDraft}
      />
    );
    const { rerender } = render(view("invite", "shared"));

    fireEvent.click(within(propertyCard("join-rule")).getByRole("radio", { name: /Public/ }));
    expect(onSetAccessDraft).toHaveBeenCalledWith({ kind: "rule", scope, rule: "public" });
    rerender(view("public", "shared"));
    fireEvent.click(within(propertyCard("join-rule")).getByRole("button", { name: "Save join rule" }));
    expect(onUpdateRoomSetting).toHaveBeenNthCalledWith(1, baseRoom.room_id, {
      accessPolicy: { rule: "public", allowTargets: [] }
    });

    fireEvent.click(
      within(propertyCard("history-visibility")).getByRole("radio", { name: /Since invite/ })
    );
    rerender(view("public", "invited"));
    fireEvent.click(
      within(propertyCard("history-visibility")).getByRole("button", {
        name: "Save history visibility"
      })
    );
    expect(onUpdateRoomSetting).toHaveBeenNthCalledWith(2, baseRoom.room_id, {
      historyVisibility: "invited"
    });
  });

  test("shows a join rule it cannot set as itself rather than as another rule", () => {
    render(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        roomManagement={{
          selected_room_id: baseRoom.room_id,
          settings: {
            room_id: baseRoom.room_id,
            name: "Alpha Room",
            topic: null,
            avatar_url: null,
            join_rule: "knockRestricted",
            history_visibility: "shared",
            permissions: {
              can_edit_settings: true,
              can_change_join_rule: true,
              can_edit_roles: true,
              can_invite: true,
              can_kick: true,
              can_ban: true,
              can_unban: true
            },
            members: []
          },
          operation: { kind: "idle" }
        }}
        onUpdateRoomSetting={vi.fn()}
      />
    );

    expect(propertyCard("join-rule").textContent).toContain(t("room.joinRuleKnockRestricted"));
    const current = within(propertyCard("join-rule")).getByRole("radio", {
      name: /Knock or restricted/
    }) as HTMLInputElement;
    expect(current.disabled).toBe(true);
    expect(current.checked).toBe(true);
    // The membership route is offered; the unmodelled rule is shown as itself.
    expect(
      within(propertyCard("join-rule")).getByRole("radio", { name: /Members of a Space/ })
    ).toBeTruthy();
  });

  test("ignores a draft that belongs to another editor's scope", () => {
    render(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        roomManagement={{
          selected_room_id: baseRoom.room_id,
          settings: roomSettings(),
          draft: { scope: { kind: "create", sessionId: 0 }, revision: 1, rule: "public" },
          operation: { kind: "idle" }
        }}
        accessPreview={accessPreviewFixture("access", true)}
        onUpdateRoomSetting={vi.fn()}
        onSetAccessDraft={vi.fn()}
      />
    );
    const card = propertyCard("join-rule");
    // The confirmed invite rule is selected; the create draft's public rule is not.
    expect(
      (within(card).getByRole("radio", { name: /Invite only/ }) as HTMLInputElement).checked
    ).toBe(true);
    expect(
      (within(card).getByRole("radio", { name: /Public/ }) as HTMLInputElement).checked
    ).toBe(false);
  });

  test("enables Save only for a valid real access change", () => {
    const onUpdateRoomSetting = vi.fn();
    const scope = { kind: "room" as const, roomId: baseRoom.room_id };
    const view = (allowTargets: string[], confirmed: boolean) => (
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
        spaces={[spaceSummary("!space:example.invalid", "Design")]}
        roomManagement={{
          selected_room_id: baseRoom.room_id,
          settings: roomSettings(),
          draft: { scope, revision: 1, rule: "restricted", allowTargets },
          operation: { kind: "idle" }
        }}
        accessPreview={{ ...accessPreviewFixture("access", confirmed), scope }}
        onUpdateRoomSetting={onUpdateRoomSetting}
        onSetAccessDraft={vi.fn()}
      />
    );
    const save = () =>
      within(propertyCard("join-rule")).getByRole("button", { name: "Save join rule" });

    // Reordered/identical server list is not a change: Rust's preview says
    // confirmed, so Save is disabled.
    const { rerender } = render(view(["!space:example.invalid"], true));
    expect((save() as HTMLButtonElement).disabled).toBe(true);

    // A genuinely changed allow list is a valid real change.
    rerender(view(["!space:example.invalid", "!space2:example.invalid"], false));
    expect((save() as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(save());
    expect(onUpdateRoomSetting).toHaveBeenCalledWith(baseRoom.room_id, {
      accessPolicy: {
        rule: "restricted",
        allowTargets: ["!space:example.invalid", "!space2:example.invalid"]
      }
    });

    // An explicitly empty allow list is not a submittable route.
    rerender(view([], false));
    expect((save() as HTMLButtonElement).disabled).toBe(true);
  });

  test("shows current access and history while disabling edits without permission", () => {
    render(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        roomManagement={{
          selected_room_id: baseRoom.room_id,
          settings: {
            room_id: baseRoom.room_id,
            name: "Alpha Room",
            topic: null,
            avatar_url: null,
            join_rule: "invite",
            history_visibility: "joined",
            permissions: {
              can_edit_settings: false,
              can_change_join_rule: false,
              can_edit_roles: false,
              can_invite: false,
              can_kick: false,
              can_ban: false,
              can_unban: false
            },
            members: []
          },
          operation: { kind: "idle" }
        }}
      />
    );

    // Read-only in place: the value and the reason share the card, and there
    // is no disabled form elsewhere to find.
    expect(screen.queryByRole("combobox", { name: "Join rule" })).toBeNull();
    expect(screen.queryByRole("combobox", { name: "History visibility" })).toBeNull();
    for (const property of ["topic", "avatar", "join-rule", "history-visibility"]) {
      const card = propertyCard(property);
      expect(within(card).queryByRole("button")).toBeNull();
      expect(card.textContent).toContain(t("room.settingNoPermission"));
    }
    expect(propertyCard("join-rule").textContent).toContain("Invite only");
    expect(propertyCard("history-visibility").textContent).toContain("Since join");
  });

  test("lets an account change the join rule without the aggregate settings permission (#1220)", () => {
    const onUpdateRoomSetting = vi.fn();
    render(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        roomManagement={{
          selected_room_id: baseRoom.room_id,
          settings: {
            room_id: baseRoom.room_id,
            name: "Alpha Room",
            topic: null,
            avatar_url: null,
            join_rule: "invite",
            history_visibility: "shared",
            permissions: {
              can_edit_settings: false,
              can_change_join_rule: true,
              can_edit_roles: false,
              can_invite: false,
              can_kick: false,
              can_ban: false,
              can_unban: false
            },
            members: []
          },
          draft: {
            scope: { kind: "room", roomId: baseRoom.room_id },
            revision: 1,
            rule: "public"
          },
          operation: { kind: "idle" }
        }}
        accessPreview={accessPreviewFixture("access", false)}
        onUpdateRoomSetting={onUpdateRoomSetting}
      />
    );

    // The event-specific fact admits the join-rule control; the aggregate keeps
    // the room name and history read-only in place.
    fireEvent.click(within(propertyCard("join-rule")).getByRole("radio", { name: /Public/ }));
    fireEvent.click(within(propertyCard("join-rule")).getByRole("button", { name: "Save join rule" }));
    expect(onUpdateRoomSetting).toHaveBeenCalledWith(baseRoom.room_id, {
      accessPolicy: { rule: "public", allowTargets: [] }
    });
    for (const property of ["topic", "history-visibility"]) {
      const card = propertyCard(property);
      expect(within(card).queryByRole("button")).toBeNull();
      expect(card.textContent).toContain(t("room.settingNoPermission"));
    }
  });

  test("keeps a room-name composition across equivalent Rust settings snapshots", () => {
    const management = () => ({
      selected_room_id: baseRoom.room_id,
      settings: {
        room_id: baseRoom.room_id,
        name: "Alpha Room",
        topic: null,
        avatar_url: null,
        join_rule: "invite" as const,
        history_visibility: "shared" as const,
        permissions: {
          can_edit_settings: true,
          can_change_join_rule: true,
          can_edit_roles: true,
          can_invite: true,
          can_kick: true,
          can_ban: true,
          can_unban: true
        },
        members: []
      },
      operation: { kind: "idle" as const }
    });
    const view = () => (
      <RoomInfoPanel
        room={baseRoom}
        roomManagement={management()}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        onUpdateRoomSetting={vi.fn()}
      />
    );
    const { rerender } = render(view());
    const name = screen.getByRole("textbox", { name: "Room name" }) as HTMLInputElement;

    fireEvent.compositionStart(name);
    fireEvent.change(name, { target: { value: "日本語変換中" } });
    name.setSelectionRange(3, 5);
    rerender(view());

    expect(name.value).toBe("日本語変換中");
    expect([name.selectionStart, name.selectionEnd]).toEqual([3, 5]);
  });

  test("renders room identity and simplified info entries", () => {
    render(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
        spaces={[
          {
            space_id: "!space-work:example.invalid",
            raw_name: null,
            display_name: "Synthetic Workspace",
            avatar: null,
            join_rule: null,
            child_room_ids: ["!room-alpha:example.invalid"],
            parent_side_child_room_ids: ["!room-alpha:example.invalid"]
          }
        ]}
      />
    );

    expect(screen.getByText("Alpha Room")).toBeTruthy();
    expect(screen.getByText("!room-alpha:example.invalid")).toBeTruthy();
    expect(screen.getByRole("button", { name: "People" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Files" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Notifications" })).toBeTruthy();
    // Issue #1008: no entry without a destination.
    expect(screen.queryByRole("button", { name: "Room settings" })).toBeNull();
    expect(screen.getByText("Synthetic Workspace")).toBeTruthy();
  });

  test.each(["en", "ja"] as const)("renders and copies Rust share URLs in %s", async (locale) => {
    setActiveLocaleProfile(locale, "none");
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    const panel = (alias: string | null, link: string) => (
      <RoomInfoPanel
        room={{ ...baseRoom, is_encrypted: true }}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        roomManagement={{
          selected_room_id: "!room-alpha:example.invalid",
          settings: {
            room_id: "!room-alpha:example.invalid",
            name: "Alpha Room",
            topic: null,
            avatar_url: null,
            join_rule: "public",
            history_visibility: "worldReadable",
            permissions: {
              can_edit_settings: true,
              can_change_join_rule: true,
              can_edit_roles: true,
              can_invite: true,
              can_kick: true,
              can_ban: true,
              can_unban: false
            },
            canonical_alias: alias,
            alternate_aliases: [],
            share_link: link,
            members: []
          },
          operation: { kind: "idle" }
        }}
      />
    );

    const { rerender } = render(panel("#alpha:example.invalid", "https://matrix.to/#/%23alpha%3Aexample.invalid"));
    if (locale === "en") {
      const status = screen.getByLabelText("Room status");
      expect(status.textContent).toContain("Encrypted");
      expect(status.textContent).toContain("Public");
      expect(status.textContent).toContain("Anyone can see history");
    }
    expect(screen.getByText("https://matrix.to/#/%23alpha%3Aexample.invalid")).toBeTruthy();
    await act(async () => fireEvent.click(screen.getByRole("button", { name: t("room.copyShareLink") })));
    expect(writeText).toHaveBeenCalledWith("https://matrix.to/#/%23alpha%3Aexample.invalid");
    expect(screen.getByRole("status").textContent).toBe(t("room.shareLinkCopied"));
    writeText.mockRejectedValueOnce(new Error("synthetic failure"));
    await act(async () => fireEvent.click(screen.getByRole("button", { name: t("room.copyShareLink") })));
    expect(screen.getByRole("status").textContent).toBe(t("room.shareLinkCopyFailed"));

    const changedLink = "https://matrix.to/#/%23changed%3Aexample.invalid";
    rerender(panel("#changed:example.invalid", changedLink));
    expect(screen.queryByText("#alpha:example.invalid")).toBeNull();
    expect(screen.getByText("#changed:example.invalid")).toBeTruthy();
    expect(screen.getByText(changedLink)).toBeTruthy();
    expect(screen.queryByRole("status")).toBeNull();

    let finishCopy!: () => void;
    writeText.mockImplementationOnce(() => new Promise<void>(resolve => { finishCopy = resolve; }));
    fireEvent.click(screen.getByRole("button", { name: t("room.copyShareLink") }));
    const roomIdLink = "https://matrix.to/#/!room-alpha:example.invalid?via=example.invalid";
    rerender(panel(null, roomIdLink));
    await act(async () => finishCopy());
    expect(screen.queryByRole("status")).toBeNull();
    expect(screen.queryByText("#changed:example.invalid")).toBeNull();
    expect(screen.getByText(roomIdLink)).toBeTruthy();
    await act(async () => fireEvent.click(screen.getByRole("button", { name: t("room.copyShareLink") })));
    expect(writeText).toHaveBeenLastCalledWith(roomIdLink);
    expect(screen.getByRole("status").textContent).toBe(t("room.shareLinkCopied"));
  });

  test("labels direct messages distinctly from rooms", () => {
    render(
      <RoomInfoPanel
        room={{
          ...baseRoom,
          room_id: "!dm-alice:example.invalid",
          display_name: "Alice",
          display_label: "Alice",
          original_display_label: "Alice",
          is_dm: true,
          dm_user_ids: ["@alice:example.invalid"],
          parent_space_ids: [],
          dm_space_ids: [],
          unread_count: 0
        }}
        roomNotificationSettings={idleSettings}
        spaces={[]}
      />
    );

    expect(screen.getByText("Direct message")).toBeTruthy();
    expect(screen.queryAllByText("No Spaces").length).toBeGreaterThanOrEqual(1);
  });

  test("renders room titles from the Rust-projected display label", () => {
    render(
      <RoomInfoPanel
        room={{
          ...baseRoom,
          room_id: "!dm-alice:example.invalid",
          display_name: "Alice Upstream",
          display_label: "Alice Local",
          original_display_label: "Alice Upstream",
          is_dm: true,
          dm_user_ids: ["@alice:example.invalid"],
          parent_space_ids: [],
          dm_space_ids: [],
          unread_count: 0
        }}
        roomNotificationSettings={idleSettings}
        spaces={[]}
      />
    );

    expect(screen.getByText("Alice Local")).toBeTruthy();
    expect(screen.queryByText("Alice Upstream")).toBeNull();
  });

  test("opens the People panel when the People entry is clicked", () => {
    const onOpenPeople = vi.fn();
    render(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        onOpenPeople={onOpenPeople}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "People" }));
    expect(onOpenPeople).toHaveBeenCalledTimes(1);
  });

  test("does not render a dense member list in the room info panel", () => {
    render(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        roomManagement={{
          selected_room_id: "!room-alpha:example.invalid",
          settings: {
            room_id: "!room-alpha:example.invalid",
            name: "Alpha Room",
            topic: null,
            avatar_url: null,
            join_rule: "invite",
            history_visibility: "shared",
            permissions: {
              can_edit_settings: true,
              can_change_join_rule: true,
              can_edit_roles: true,
              can_invite: true,
              can_kick: true,
              can_ban: true,
              can_unban: false
            },
            members: [
              {
                user_id: "@member:example.invalid",
                display_name: "Upstream Member",
                display_label: "Local Remark",
                original_display_label: "Upstream Member",
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

    expect(screen.queryByText("Local Remark")).toBeNull();
    expect(screen.queryByRole("button", { name: /Message/ })).toBeNull();
  });

  test("renders notification mode options", () => {
    render(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        onSetRoomNotificationMode={() => undefined}
      />
    );

    expect(screen.getByText("All messages")).toBeTruthy();
    expect(screen.getByText("Mentions only")).toBeTruthy();
    expect(screen.getByText("Mute")).toBeTruthy();
  });

  test("confirms only stock outbound-session rotation for encrypted rooms", async () => {
    const onForceRotateOutboundSession = vi.fn();
    render(
      <RoomInfoPanel
        room={{ ...baseRoom, is_encrypted: true }}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        onForceRotateOutboundSession={onForceRotateOutboundSession}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "Force encryption key rotation" }));
    expect(onForceRotateOutboundSession).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Confirm rotation" }));

    expect(onForceRotateOutboundSession).toHaveBeenCalledWith(baseRoom.room_id);
    expect(
      await screen.findByText("Current outbound key discarded. The next message will rotate normally.")
    ).toBeTruthy();
    expect(screen.queryByText(/share index 0/i)).toBeNull();
    expect(screen.queryByText(/reshare/i)).toBeNull();
  });

  test("ignores a forced-rotation completion after switching rooms", async () => {
    let resolveRotation: () => void = () => {};
    const onForceRotateOutboundSession = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          resolveRotation = () => resolve();
        })
    );
    const { rerender } = render(
      <RoomInfoPanel
        room={{ ...baseRoom, is_encrypted: true }}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        onForceRotateOutboundSession={onForceRotateOutboundSession}
      />
    );
    fireEvent.click(screen.getByRole("button", { name: "Force encryption key rotation" }));
    fireEvent.click(screen.getByRole("button", { name: "Confirm rotation" }));

    rerender(
      <RoomInfoPanel
        room={{ ...baseRoom, room_id: "!room-beta:example.invalid", is_encrypted: true }}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        onForceRotateOutboundSession={onForceRotateOutboundSession}
      />
    );
    await act(async () => resolveRotation());

    expect(screen.queryByText(/Current outbound key discarded/)).toBeNull();
  });

  test("requests non-destructive room timeline repair", () => {
    const onRepairRoomTimeline = vi.fn();

    render(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        onRepairRoomTimeline={onRepairRoomTimeline}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "Repair room timeline" }));

    expect(onRepairRoomTimeline).toHaveBeenCalledWith("!room-alpha:example.invalid");
  });

  test("selects the current notification mode", () => {
    const { container } = render(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={{
          mode: { kind: "mentions" },
          operation: { kind: "idle" }
        }}
        spaces={[]}
        onSetRoomNotificationMode={() => undefined}
      />
    );

    const select = container.querySelector("select");
    expect(select).toBeTruthy();
    expect(select?.value).toBe("mentions");
  });

  test("disables the notification select while a mode change is pending", () => {
    const { container } = render(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={pendingSettings}
        spaces={[]}
        onSetRoomNotificationMode={() => undefined}
      />
    );

    const select = container.querySelector("select");
    expect(select).toBeTruthy();
    expect(select?.hasAttribute("disabled")).toBe(true);
    expect(select?.value).toBe("mute");
  });

  test("disables the notification select when no handler is provided", () => {
    const { container } = render(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
        spaces={[]}
      />
    );

    const select = container.querySelector("select");
    expect(select).toBeTruthy();
    expect(select?.hasAttribute("disabled")).toBe(true);
  });
});

describe("RoomInfoPanel URL previews", () => {
  test("renders the URL-preview section when settings and handler are supplied", () => {
    render(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        appSettings={baseAppSettings}
        linkPreviewSettings={baseLinkPreviewSettings}
        onSetRoomUrlPreviewOverride={() => undefined}
      />
    );

    expect(
      screen.getByRole("switch", { name: "Enable link previews for this room" })
    ).toBeTruthy();
  });

  test("checks the toggle for an unencrypted room that falls back to the global setting", () => {
    render(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        appSettings={baseAppSettings}
        linkPreviewSettings={baseLinkPreviewSettings}
        onSetRoomUrlPreviewOverride={() => undefined}
      />
    );

    const toggle = screen.getByRole("switch", {
      name: "Enable link previews for this room"
    });
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    expect(toggle.hasAttribute("disabled")).toBe(false);
  });

  test("unchecks but keeps the toggle enabled for encrypted rooms by default", () => {
    render(
      <RoomInfoPanel
        room={{ ...baseRoom, is_encrypted: true }}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        appSettings={baseAppSettings}
        linkPreviewSettings={baseLinkPreviewSettings}
        onSetRoomUrlPreviewOverride={() => undefined}
      />
    );

    const toggle = screen.getByRole("switch", {
      name: "Enable link previews for this room"
    });
    expect(toggle.getAttribute("aria-checked")).toBe("false");
    expect(toggle.hasAttribute("disabled")).toBe(false);
    expect(
      screen.getByText(
        "Encrypted-room previews can reveal URLs to the homeserver and destination site."
      )
    ).toBeTruthy();
  });

  test("checks the toggle for encrypted rooms when the encrypted global default is on", () => {
    render(
      <RoomInfoPanel
        room={{ ...baseRoom, is_encrypted: true }}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        appSettings={{
          ...baseAppSettings,
          values: {
            ...baseAppSettings.values,
            display: {
              ...baseAppSettings.values.display,
              encrypted_url_previews_enabled: true
            }
          }
        }}
        linkPreviewSettings={baseLinkPreviewSettings}
        onSetRoomUrlPreviewOverride={() => undefined}
      />
    );

    const toggle = screen.getByRole("switch", {
      name: "Enable link previews for this room"
    });
    expect(toggle.getAttribute("aria-checked")).toBe("true");
  });

  test("dispatches a per-room override when the toggle is clicked", () => {
    const onSetRoomUrlPreviewOverride = vi.fn();
    render(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        appSettings={baseAppSettings}
        linkPreviewSettings={baseLinkPreviewSettings}
        onSetRoomUrlPreviewOverride={onSetRoomUrlPreviewOverride}
      />
    );

    const toggle = screen.getByRole("switch", {
      name: "Enable link previews for this room"
    });
    fireEvent.click(toggle);

    expect(onSetRoomUrlPreviewOverride).toHaveBeenCalledTimes(1);
    expect(onSetRoomUrlPreviewOverride).toHaveBeenCalledWith(
      "!room-alpha:example.invalid",
      false
    );
  });
});

function roomSettings(overrides: Partial<RoomSettingsSnapshot> = {}): RoomSettingsSnapshot {
  return {
    room_id: baseRoom.room_id,
    name: "Alpha Room",
    topic: "Original topic",
    avatar_url: null,
    join_rule: "invite",
    history_visibility: "shared",
    permissions: {
      can_edit_settings: true,
      can_change_join_rule: true,
      can_edit_roles: true,
      can_invite: true,
      can_kick: true,
      can_ban: true,
      can_unban: true
    },
    members: [],
    ...overrides
  };
}

function spaceSummary(spaceId: string, displayName: string): SpaceSummary {
  return {
    space_id: spaceId,
    raw_name: displayName,
    display_name: displayName,
    avatar: null,
    join_rule: "invite",
    child_room_ids: [],
    parent_side_child_room_ids: []
  };
}

function managed(
  settings: RoomSettingsSnapshot = roomSettings(),
  operation: RoomManagementOperationState = { kind: "idle" }
): RoomManagementState {
  return { selected_room_id: settings.room_id, settings, operation };
}

function propertyCard(property: string): HTMLElement {
  const card = document.querySelector(`[data-setting-property="${property}"]`);
  if (!card) throw new Error(`no ${property} card`);
  return card as HTMLElement;
}

/** A Rust access/history preview fixture (#1177). */
function accessPreviewFixture(
  context: "access" | "history",
  confirmed: boolean
): RoomAccessPreview {
  return {
    scope: { kind: "room", roomId: baseRoom.room_id },
    context,
    confirmed,
    canonicalPolicyKey:
      context === "access" ? "restricted|!space-b:example.invalid" : undefined,
    outcome: {
      join: { messageId: "room.accessOutcomeJoinInvite" },
      history: { messageId: "room.accessOutcomeHistoryShared" },
      encryption: { messageId: "room.accessOutcomeNotEncrypted" },
      directory: { messageId: "room.accessOutcomeDirectoryPrivate" },
      nonRetroactive: { messageId: "room.historyNonRetroactive" }
    }
  };
}

// Issue #1008: each property's value, change control and result share one card.
describe("RoomInfoPanel property cards", () => {
  test("shows the topic once and edits it in the same card", () => {
    const onUpdateRoomSetting = vi.fn();
    render(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        roomManagement={managed()}
        onUpdateRoomSetting={onUpdateRoomSetting}
      />
    );

    expect(screen.queryByText("Current topic")).toBeNull();
    expect(screen.getAllByText("Original topic")).toHaveLength(1);
    expect(screen.queryByRole("textbox", { name: "Room topic" })).toBeNull();
    const card = propertyCard("topic");
    expect(card.textContent).toContain("Original topic");

    fireEvent.click(within(card).getByRole("button", { name: "Edit topic" }));
    const topic = within(card).getByRole("textbox", { name: "Room topic" }) as HTMLTextAreaElement;
    expect(document.activeElement).toBe(topic);
    expect(topic.value).toBe("Original topic");
    fireEvent.change(topic, { target: { value: "  Updated topic  " } });
    fireEvent.click(within(card).getByRole("button", { name: "Save topic" }));

    expect(onUpdateRoomSetting).toHaveBeenCalledWith(baseRoom.room_id, { topic: "Updated topic" });
    expect(within(propertyCard("topic")).queryByRole("textbox")).toBeNull();
    expect(document.activeElement).toBe(within(propertyCard("topic")).getByRole("heading", { name: "Topic" }));
  });

  test("shows the avatar preview with its address and edits it in the same card", () => {
    const onUpdateRoomSetting = vi.fn();
    render(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        roomManagement={managed(roomSettings({ avatar_url: "mxc://example.invalid/avatar" }))}
        onUpdateRoomSetting={onUpdateRoomSetting}
      />
    );

    expect(screen.queryByText("Current avatar")).toBeNull();
    const card = propertyCard("avatar");
    expect(card.querySelector(".settings-property-avatar")).toBeTruthy();
    expect(screen.getAllByText("mxc://example.invalid/avatar")).toHaveLength(1);

    fireEvent.click(within(card).getByRole("button", { name: "Edit avatar" }));
    const field = within(card).getByRole("textbox", { name: "Room avatar URL" });
    fireEvent.change(field, { target: { value: "" } });
    fireEvent.click(within(card).getByRole("button", { name: "Save avatar" }));
    expect(onUpdateRoomSetting).toHaveBeenCalledWith(baseRoom.room_id, { avatarUrl: null });
  });

  test("cancel re-seeds the confirmed policy and returns focus to the heading", () => {
    const onUpdateRoomSetting = vi.fn();
    const onSetAccessDraft = vi.fn();
    const scope = { kind: "room" as const, roomId: baseRoom.room_id };
    render(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        roomManagement={{
          ...managed(),
          draft: { scope, revision: 1, rule: "public" }
        }}
        accessPreview={accessPreviewFixture("access", false)}
        onUpdateRoomSetting={onUpdateRoomSetting}
        onSetAccessDraft={onSetAccessDraft}
      />
    );

    const card = propertyCard("join-rule");
    fireEvent.click(within(card).getByRole("radio", { name: /Public/ }));
    fireEvent.click(within(card).getByRole("button", { name: "Cancel" }));

    expect(onUpdateRoomSetting).not.toHaveBeenCalled();
    // Rust re-seeds the draft from the confirmed policy, so the visible allow
    // selection returns too (not only the rule and outcome).
    expect(onSetAccessDraft).toHaveBeenCalledWith({ kind: "open", scope });
    expect(document.activeElement).toBe(within(card).getByRole("heading", { level: 4 }));
  });

  test("restores the confirmed allow list and toggles each target against the current draft", () => {
    const onSetAccessDraft = vi.fn();
    const scope = { kind: "room" as const, roomId: baseRoom.room_id };
    render(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
        spaces={[
          spaceSummary("!space-a:example.invalid", "Alpha"),
          spaceSummary("!space-b:example.invalid", "Beta")
        ]}
        roomManagement={{
          selected_room_id: baseRoom.room_id,
          settings: roomSettings({ join_rule: "restricted" }),
          // Rust seeded the confirmed target A into the draft.
          draft: { scope, revision: 0, rule: "restricted", allowTargets: ["!space-a:example.invalid"] },
          operation: { kind: "idle" }
        }}
        accessPreview={accessPreviewFixture("access", true)}
        onUpdateRoomSetting={vi.fn()}
        onSetAccessDraft={onSetAccessDraft}
      />
    );

    const card = propertyCard("join-rule");
    // The confirmed restricted(A) selection is restored, not shown unchecked.
    expect(
      (
        within(card).getByRole("checkbox", {
          name: /Alpha/
        }) as HTMLInputElement
      ).checked
    ).toBe(true);
    expect(
      (
        within(card).getByRole("checkbox", {
          name: /Beta/
        }) as HTMLInputElement
      ).checked
    ).toBe(false);

    // Each edit carries the target and its state, never a replacement list
    // rebuilt from lagging props, so two rapid edits cannot drop one another.
    fireEvent.click(within(card).getByRole("checkbox", { name: /Beta/ }));
    expect(onSetAccessDraft).toHaveBeenNthCalledWith(1, {
      kind: "toggleAllowTarget",
      scope,
      target: "!space-b:example.invalid",
      selected: true
    });
    fireEvent.click(within(card).getByRole("checkbox", { name: /Alpha/ }));
    expect(onSetAccessDraft).toHaveBeenNthCalledWith(2, {
      kind: "toggleAllowTarget",
      scope,
      target: "!space-a:example.invalid",
      selected: false
    });
  });

  test("a same-rule allow-list save is saved only when Rust confirms the full policy", () => {
    const onUpdateRoomSetting = vi.fn();
    const scope = { kind: "room" as const, roomId: baseRoom.room_id };
    const view = (confirmed: boolean) => (
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
        spaces={[
          spaceSummary("!space-a:example.invalid", "Alpha"),
          spaceSummary("!space-b:example.invalid", "Beta")
        ]}
        roomManagement={{
          selected_room_id: baseRoom.room_id,
          // The confirmed scalar rule is already `restricted`: a scalar
          // comparison alone would read `Saved` for the allow-list edit.
          settings: roomSettings({ join_rule: "restricted" }),
          draft: {
            scope,
            revision: 1,
            rule: "restricted",
            allowTargets: ["!space-b:example.invalid"]
          },
          operation: { kind: "idle" }
        }}
        accessPreview={accessPreviewFixture("access", confirmed)}
        onUpdateRoomSetting={onUpdateRoomSetting}
        onSetAccessDraft={vi.fn()}
      />
    );

    const { rerender } = render(view(false));
    const card = propertyCard("join-rule");
    expect(within(card).queryByRole("status")).toBeNull();

    fireEvent.click(within(card).getByRole("button", { name: "Save join rule" }));
    expect(onUpdateRoomSetting).toHaveBeenCalledWith(baseRoom.room_id, {
      accessPolicy: { rule: "restricted", allowTargets: ["!space-b:example.invalid"] }
    });

    // The same-rule allow-list edit is not yet saved while Rust reports the
    // draft as unconfirmed.
    rerender(view(false));
    expect(within(propertyCard("join-rule")).queryByRole("status")).toBeNull();

    // Only Rust confirming the full canonical policy turns it to Saved.
    rerender(view(true));
    expect(within(propertyCard("join-rule")).getByRole("status").textContent).toBe(
      "Saved"
    );
  });

  test("history notes explain the value being chosen, in its card", () => {
    const scope = { kind: "room" as const, roomId: baseRoom.room_id };
    const historyPreviewFor = (visibility: RoomHistoryVisibility): RoomAccessPreview => ({
      scope,
      context: "history",
      confirmed: false,
      outcome: {
        join: { messageId: "room.accessOutcomeJoinInvite" },
        history:
          visibility === "worldReadable"
            ? { messageId: "room.accessOutcomeHistoryWorldReadable" }
            : { messageId: "room.accessOutcomeHistoryShared" },
        encryption: { messageId: "room.accessOutcomeEncrypted" },
        directory: { messageId: "room.accessOutcomeDirectoryPrivate" },
        ...(visibility === "shared" || visibility === "invited"
          ? { historyKeyCaveat: { messageId: "room.historySharedEncryptedHint" as const } }
          : {}),
        nonRetroactive: { messageId: "room.historyNonRetroactive" }
      }
    });
    const view = (visibility: RoomHistoryVisibility) => (
      <RoomInfoPanel
        room={{ ...baseRoom, is_encrypted: true }}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        roomManagement={{
          ...managed(),
          draft: { scope, revision: 1, history: visibility }
        }}
        historyPreview={historyPreviewFor(visibility)}
        onUpdateRoomSetting={vi.fn()}
      />
    );

    const { rerender } = render(view("shared"));
    const card = propertyCard("history-visibility");
    expect(card.textContent).toContain(t("room.historySharedDescription"));
    expect(card.textContent).toContain(t("room.historySharedEncryptedHint"));
    expect(card.textContent).toContain(t("room.historyNonRetroactive"));

    rerender(view("worldReadable"));
    expect(card.textContent).toContain(t("room.historyWorldReadableWarning"));
    expect(card.textContent).toContain(t("room.historyNonRetroactive"));
    expect(card.textContent).not.toContain(t("room.historySharedEncryptedHint"));
  });

  test("attributes Rust's pending, failed and saved state to the submitted property only", () => {
    const onUpdateRoomSetting = vi.fn();
    const view = (management: RoomManagementState) => (
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        roomManagement={management}
        onUpdateRoomSetting={onUpdateRoomSetting}
      />
    );
    const { rerender } = render(view(managed()));

    fireEvent.click(screen.getByRole("button", { name: "Edit topic" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Room topic" }), {
      target: { value: "Updated topic" }
    });
    fireEvent.click(screen.getByRole("button", { name: "Save topic" }));

    rerender(
      view(
        managed(roomSettings(), {
          kind: "pending",
          request_id: 7,
          room_id: baseRoom.room_id,
          operation: "settings"
        })
      )
    );
    expect(within(propertyCard("topic")).getByRole("status").textContent).toBe("Saving…");
    expect(within(propertyCard("avatar")).queryByRole("status")).toBeNull();
    expect(within(propertyCard("join-rule")).queryByRole("status")).toBeNull();
    // Nothing else can be submitted while Rust holds the change.
    expect(screen.getByRole("button", { name: "Edit avatar" })).toHaveProperty("disabled", true);

    rerender(
      view(
        managed(roomSettings(), {
          kind: "failed",
          request_id: 7,
          room_id: baseRoom.room_id,
          operation: "settings",
          failureKind: "forbidden"
        })
      )
    );
    expect(within(propertyCard("topic")).getByRole("status").textContent).toBe(
      t("room.settingForbidden")
    );
    // A failure is not a success: the confirmed value is still the old one.
    expect(propertyCard("topic").textContent).toContain("Original topic");
    expect(within(propertyCard("avatar")).queryByRole("status")).toBeNull();

    // Retry from the same card.
    fireEvent.click(screen.getByRole("button", { name: "Edit topic" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Room topic" }), {
      target: { value: "Updated topic" }
    });
    fireEvent.click(screen.getByRole("button", { name: "Save topic" }));
    expect(onUpdateRoomSetting).toHaveBeenCalledTimes(2);
    // The earlier failure is not this submission's outcome.
    expect(within(propertyCard("topic")).queryByRole("status")).toBeNull();

    rerender(view(managed(roomSettings({ topic: "Updated topic" }))));
    expect(within(propertyCard("topic")).getByRole("status").textContent).toBe("Saved");
    expect(propertyCard("topic").textContent).toContain("Updated topic");
  });

  test("switching rooms drops an open editor and the previous room's result", () => {
    const onUpdateRoomSetting = vi.fn();
    const otherRoom = { ...baseRoom, room_id: "!room-beta:example.invalid", display_label: "Beta Room" };
    const { rerender } = render(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        roomManagement={managed()}
        onUpdateRoomSetting={onUpdateRoomSetting}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "Edit topic" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Room topic" }), {
      target: { value: "Draft for alpha" }
    });
    rerender(
      <RoomInfoPanel
        room={otherRoom}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        roomManagement={managed(roomSettings({ room_id: otherRoom.room_id, topic: "Beta topic" }), {
          kind: "failed",
          request_id: 3,
          room_id: otherRoom.room_id,
          operation: "settings",
          failureKind: "network"
        })}
        onUpdateRoomSetting={onUpdateRoomSetting}
      />
    );

    expect(screen.queryByRole("textbox", { name: "Room topic" })).toBeNull();
    expect(propertyCard("topic").textContent).toContain("Beta topic");
    expect(within(propertyCard("topic")).queryByRole("status")).toBeNull();
    expect(onUpdateRoomSetting).not.toHaveBeenCalled();
  });

  test("summary badges lead to the setting they summarize", () => {
    render(
      <RoomInfoPanel
        room={baseRoom}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        roomManagement={managed(roomSettings({ join_rule: "public", history_visibility: "worldReadable" }))}
        onUpdateRoomSetting={vi.fn()}
      />
    );

    fireEvent.click(
      screen.getByRole("button", { name: t("room.statusShowSetting", { status: t("room.statusPublic") }) })
    );
    expect(document.activeElement).toBe(
      within(propertyCard("join-rule")).getByRole("heading", { name: "Join rule" })
    );
    fireEvent.click(
      screen.getByRole("button", {
        name: t("room.statusShowSetting", { status: t("room.statusHistoryWorldReadable") })
      })
    );
    expect(document.activeElement).toBe(
      within(propertyCard("history-visibility")).getByRole("heading", { name: "History visibility" })
    );
    // Encryption cannot be changed here, so its badge is not a link.
    expect(screen.queryByRole("button", { name: /Not encrypted/ })).toBeNull();
  });

  test("the Notifications entry leads to the notification setting", () => {
    render(
      <RoomInfoPanel room={baseRoom} roomNotificationSettings={idleSettings} spaces={[]} />
    );

    fireEvent.click(screen.getByRole("button", { name: "Notifications" }));
    expect(document.activeElement).toBe(
      within(screen.getByRole("region", { name: "Notifications" })).getByRole("heading")
    );
  });

  test("keeps download, repair and diagnostics after the room's properties", () => {
    render(
      <RoomInfoPanel
        room={{ ...baseRoom, is_encrypted: true }}
        roomNotificationSettings={idleSettings}
        spaces={[]}
        roomManagement={managed()}
        onUpdateRoomSetting={vi.fn()}
        onRepairRoomTimeline={vi.fn()}
        onForceRotateOutboundSession={vi.fn()}
      />
    );

    const details = screen.getByRole("region", { name: "Details" });
    const access = screen.getByRole("region", { name: t("room.accessAndHistory") });
    const permissions = screen.getByRole("region", { name: t("room.rolePermissions") });
    expect(details.nextElementSibling).toBe(access);
    for (const auxiliary of [t("room.repair"), t("room.encryptionDebugging")]) {
      const section = screen.getByRole("region", { name: auxiliary });
      expect(permissions.compareDocumentPosition(section) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    }
  });
});
