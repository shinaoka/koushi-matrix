// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";
import type { RoomManagementState, SpaceSummary } from "../domain/types";
import { SpaceInfoPanel } from "./SpaceInfoPanel";

afterEach(cleanup);
const ID = "!description:example.invalid";
const space: SpaceSummary = { space_id: ID, raw_name: "Workspace", display_name: "Workspace", avatar: null, join_rule: "invite", topic: null, child_room_ids: [], parent_side_child_room_ids: [] };
function management(topic: string | null, canChange = true): RoomManagementState {
  return { selected_room_id: ID, operation: { kind: "idle" }, settings: {
    room_id: ID, name: "Workspace", topic, avatar_url: null, join_rule: "invite", history_visibility: "shared", members: [],
    permissions: { can_edit_settings: !canChange, can_change_topic: canChange, can_change_join_rule: false, can_edit_roles: false, can_invite: false, can_kick: false, can_ban: false, can_unban: false }
  } };
}
function card() { return screen.getByRole("group", { name: "Description" }); }
function panel(state?: RoomManagementState, onUpdateTopic = vi.fn()) {
  return <SpaceInfoPanel fallbackName="Workspace" rooms={[]} space={space} roomManagement={state} onUpdateTopic={onUpdateTopic} />;
}

describe("shared Space description", () => {
  test("distinguishes loading and an unset shared description", () => {
    const view = render(panel());
    expect(card().textContent).toContain("Loading description");
    expect(within(card()).queryByRole("button")).toBeNull();
    view.rerender(panel(management(null)));
    expect(card().textContent).toContain("No description");
    expect(card().textContent).toContain("Shared with everyone in this Space");
  });

  test("topic-only permission edits multiline text and clears through the same command", async () => {
    const update = vi.fn().mockResolvedValue(undefined);
    const view = render(panel(management("Original"), update));
    fireEvent.click(within(card()).getByRole("button", { name: "Edit description" }));
    fireEvent.change(within(card()).getByRole("textbox", { name: "Description" }), { target: { value: "Shared line\nSecond line" } });
    fireEvent.click(within(card()).getByRole("button", { name: "Save description" }));
    await act(async () => {});
    expect(update).toHaveBeenLastCalledWith(ID, "Shared line\nSecond line");
    view.rerender(panel(management("Shared line\nSecond line"), update));
    expect(card().textContent).toContain("Shared line\nSecond line");
    fireEvent.click(within(card()).getByRole("button", { name: "Clear description" }));
    await act(async () => {});
    expect(update).toHaveBeenLastCalledWith(ID, null);
  });

  test("aggregate settings permission cannot override topic denial", () => {
    render(panel(management("Read only", false)));
    expect(card().textContent).toContain("Read only");
    expect(within(card()).queryByRole("button")).toBeNull();
    expect(card().textContent).toContain("does not let you change the description");
  });
});


function startEdit(text: string) {
  fireEvent.click(within(card()).getByRole("button", { name: "Edit description" }));
  fireEvent.change(within(card()).getByRole("textbox"), { target: { value: text } });
}
function save() { fireEvent.click(within(card()).getByRole("button", { name: "Save description" })); }

test("correlated deferred failure retains its multiline draft for retry and rejects unrelated outcomes", async () => {
  let reject!: (error: Error) => void;
  const update = vi.fn().mockImplementationOnce(() => new Promise<void>((_, fail) => { reject = fail; })).mockResolvedValue(undefined);
  const view = render(panel(management("Confirmed"), update));
  startEdit("Retry line\nSecond line"); save();
  expect(within(card()).getByRole("status").textContent).toBe("Saving…");
  expect((within(card()).getByRole("button", { name: "Edit description" }) as HTMLButtonElement).disabled).toBe(true);
  const unrelated = management("Confirmed");
  unrelated.operation = { kind: "failed", request_id: 31, room_id: ID, operation: "settings", failureKind: "forbidden" };
  view.rerender(panel(unrelated, update));
  expect(within(card()).getByRole("status").textContent).toBe("Saving…");
  await act(async () => reject(new Error("synthetic failure")));
  expect(within(card()).getByRole("status").textContent).not.toBe("Saved");
  fireEvent.click(within(card()).getByRole("button", { name: "Edit description" }));
  expect((within(card()).getByRole("textbox") as HTMLTextAreaElement).value).toBe("Retry line\nSecond line");
  save(); await act(async () => {});
  expect(update).toHaveBeenLastCalledWith(ID, "Retry line\nSecond line");
  expect(within(card()).queryByText("Saved")).toBeNull();
  view.rerender(panel(management("Retry line\nSecond line"), update));
  expect(within(card()).getByRole("status").textContent).toBe("Saved");
  unrelated.settings!.topic = "Retry line\nSecond line";
  view.rerender(panel(unrelated, update));
  expect(within(card()).getByRole("status").textContent).toBe("Saved");
});

test("remote updates and candidate confirmation preserve an active multiline draft", async () => {
  const update = vi.fn();
  const view = render(panel(management("Original"), update));
  startEdit("編集中\nSecond");
  const control = within(card()).getByRole("textbox") as HTMLTextAreaElement;
  fireEvent.compositionStart(control);
  fireEvent.keyDown(control, { key: "Enter", keyCode: 229, isComposing: true });
  fireEvent.submit(control.closest("form")!);
  view.rerender(panel(management("Remote topic"), update));
  expect(control.value).toBe("編集中\nSecond");
  expect(update).not.toHaveBeenCalled();
  fireEvent.compositionEnd(control);
  await act(async () => {});
  fireEvent.click(within(card()).getByRole("button", { name: "Cancel" }));
  expect(card().textContent).toContain("Remote topic");
});

test.each(["resolve", "reject"] as const)("Space A to B to A ignores late %s and resets the editor and outcome", async (outcome) => {
  let reject!: (error: Error) => void;
  let resolve!: () => void;
  const update = vi.fn(() => new Promise<void>((ok, fail) => { resolve = ok; reject = fail; }));
  const view = render(panel(management("A confirmed"), update));
  startEdit("A unsaved"); save();
  const other = { ...space, space_id: "!other:example.invalid" };
  const state = management("B confirmed"); state.selected_room_id = other.space_id; state.settings!.room_id = other.space_id;
  view.rerender(<SpaceInfoPanel fallbackName="Other" rooms={[]} space={other} roomManagement={state} onUpdateTopic={update} />);
  expect(card().textContent).toContain("B confirmed");
  expect(within(card()).queryByRole("status")).toBeNull();
  view.rerender(panel(management("A confirmed"), update));
  await act(async () => outcome === "reject" ? reject(new Error("late synthetic failure")) : resolve());
  expect(within(card()).queryByRole("status")).toBeNull();
  fireEvent.click(within(card()).getByRole("button", { name: "Edit description" }));
  expect((within(card()).getByRole("textbox") as HTMLTextAreaElement).value).toBe("A confirmed");
});

test("wrong-target settings remain loading and a synchronous callback failure retains the draft", async () => {
  const wrong = management("Private unrelated topic"); wrong.settings!.room_id = "!wrong:example.invalid";
  const update = vi.fn(() => { throw new Error("synthetic failure"); });
  const view = render(panel(wrong, update));
  expect(card().textContent).toContain("Loading description");
  expect(card().textContent).not.toContain("Private unrelated topic");
  view.rerender(panel(management("Original"), update));
  startEdit("Retained draft"); save(); await act(async () => {});
  fireEvent.click(within(card()).getByRole("button", { name: "Edit description" }));
  expect((within(card()).getByRole("textbox") as HTMLTextAreaElement).value).toBe("Retained draft");
});
