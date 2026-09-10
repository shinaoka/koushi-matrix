// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { ProfilePanel } from "./PeoplePanel";
import { TimelineView, clearTimelineViewportSessionMemoryForTests } from "./TimelineView";
import { TimelineStoreContext } from "./timelineStoreContext";
import { applyTimelineEvent, createTimelineStore } from "../domain/timelineStore";
import { KEY, baseTransport, message } from "./timelineViewTestSupport";
import type { RoomSummary, RoomManagementState } from "../domain/types";

const room: RoomSummary = {
  room_id: "!room:example.invalid", display_name: "Room", display_label: "Room",
  original_display_label: "Room", avatar: null, is_dm: false, dm_user_ids: [],
  tags: { favourite: null, low_priority: null }, parent_space_ids: [], dm_space_ids: [],
  is_encrypted: false, unread_count: 0
};
const management: RoomManagementState = {
  selected_room_id: room.room_id, operation: { kind: "idle" }, settings: {
    room_id: room.room_id, name: "Room", topic: null, avatar_url: null,
    join_rule: "invite", history_visibility: "shared",
    permissions: { can_edit_settings: false, can_edit_roles: false, can_invite: false,
      can_kick: false, can_ban: false, can_unban: false },
    members: ["bob", "other"].map(name => ({ user_id: `@${name}:example.invalid`,
      display_name: "Reader", display_label: "Reader", original_display_label: "Reader",
      avatar_url: null, power_level: 0, role: "user", role_options: [] }))
  }
};

function profile(onSave: (id: string, alias: string | null) => void, userId = "@bob:example.invalid") {
  return <ProfilePanel userId={userId} currentUserId="@self:example.invalid"
    roomOrSpace={room} roomManagement={management} profileUsers={{}} onBack={() => undefined}
    onSetLocalUserAlias={onSave} />;
}

function editor(surface: "profile" | "timeline", onSave: (id: string, alias: string | null) => void) {
  if (surface === "profile") {
    render(profile(onSave));
    fireEvent.click(screen.getByRole("button", { name: "Set alias" }));
  } else {
    const store = applyTimelineEvent(createTimelineStore(), { InitialItems: {
      request_id: null, key: KEY, generation: 1, items: [message("$alias", "Synthetic message")]
    } });
    render(<TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
      <TimelineView timelineKey={KEY} roomId={room.room_id} transport={baseTransport({})}
        onReply={vi.fn()} onSetLocalUserAlias={onSave} />
    </TimelineStoreContext.Provider>);
    const row = screen.getByText("Synthetic message").closest("article")!;
    fireEvent.click(within(row).getByRole("button", { name: "Message actions" }));
    fireEvent.click(within(row).getByRole("menuitem", { name: "Set alias for Unknown user" }));
  }
  return screen.getByRole("textbox", { name: "Alias" });
}

afterEach(() => { cleanup(); clearTimelineViewportSessionMemoryForTests(); vi.useRealTimers(); });

for (const surface of ["profile", "timeline"] as const) {
  it(`${surface}: four input changes produce no saves until Done, then one final save`, () => {
    const save = vi.fn();
    const input = editor(surface, save);
    for (const value of ["T", "Te", "Tes", "Test"]) fireEvent.change(input, { target: { value } });
    expect(save).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Done" }));
    expect(save.mock.calls).toEqual([["@bob:example.invalid", "Test"]]);
    expect(screen.queryByRole("textbox", { name: "Alias" })).toBeNull();
  });

  it(`${surface}: IME confirmation does not save, subsequent Enter saves the composed text`, () => {
    vi.useFakeTimers();
    const save = vi.fn();
    const input = editor(surface, save);
    fireEvent.compositionStart(input);
    fireEvent.change(input, { target: { value: "別名" } });
    fireEvent.keyDown(input, { key: "Enter", keyCode: 229, isComposing: true });
    fireEvent.submit(input.closest("form")!);
    expect(save).not.toHaveBeenCalled();
    fireEvent.compositionEnd(input);
    vi.runOnlyPendingTimers();
    fireEvent.keyDown(input, { key: "Enter", keyCode: 13 });
    fireEvent.submit(input.closest("form")!);
    expect(save.mock.calls).toEqual([["@bob:example.invalid", "別名"]]);
  });

  it(`${surface}: clearing is submitted once on confirmation`, () => {
    const save = vi.fn();
    const input = editor(surface, save);
    fireEvent.change(input, { target: { value: "temporary" } });
    fireEvent.change(input, { target: { value: "   " } });
    expect(save).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Done" }));
    expect(save.mock.calls).toEqual([["@bob:example.invalid", null]]);
  });

  it(`${surface}: dismissing an unfinished draft does not save`, () => {
    const save = vi.fn();
    const input = editor(surface, save);
    fireEvent.change(input, { target: { value: "unfinished" } });
    if (surface === "profile") fireEvent.click(screen.getByRole("button", { name: "Set alias" }));
    else fireEvent.mouseDown(document.querySelector(".dialog-overlay")!);
    expect(save).not.toHaveBeenCalled();
    expect(screen.queryByRole("textbox", { name: "Alias" })).toBeNull();
  });
}

it("profile: switching users discards the previous user's unsaved draft", () => {
  const save = vi.fn();
  const view = render(profile(save));
  fireEvent.click(screen.getByRole("button", { name: "Set alias" }));
  fireEvent.change(screen.getByRole("textbox", { name: "Alias" }), { target: { value: "old draft" } });
  view.rerender(profile(save, "@other:example.invalid"));
  expect(screen.queryByRole("textbox", { name: "Alias" })).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Set alias" }));
  expect((screen.getByRole("textbox", { name: "Alias" }) as HTMLInputElement).value).toBe("");
  expect(save).not.toHaveBeenCalled();
});
