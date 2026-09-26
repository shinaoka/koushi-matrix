// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { setActiveLocaleProfile, t } from "../i18n/messages";
import type { SpaceAddRoomsModel } from "../domain/types";
import { operationFailureLabel } from "../app/uiShared";
import { AddExistingRoomDialog } from "./AddExistingRoomDialog";

/** A room version 12 room ID has no server name (#1007). */
const DOMAINLESS = "!31hneApxJ_1o-63DmFrpeqnkFfWppnzWso1JvH3ogLM";

function model(overrides: Partial<Record<string, SpaceAddRoomsModel["candidates"][number]["status"]>> = {}): SpaceAddRoomsModel {
  return {
    space_id: "!space:example.invalid",
    candidates: [
      { room_id: "!alpha:example.invalid", display_name: "Alpha notes", avatar: null, status: overrides.alpha ?? { kind: "available" } },
      { room_id: DOMAINLESS, display_name: "設計レビュー", avatar: null, status: overrides.domainless ?? { kind: "available" } },
      { room_id: "!linked:example.invalid", display_name: "Linked", avatar: null, status: { kind: "added" } }
    ]
  };
}

afterEach(() => {
  cleanup();
  setActiveLocaleProfile("en", "none");
});

function rows() {
  return within(screen.getByRole("list", { name: t("spaceAddRooms.listLabel") })).getAllByRole("listitem");
}

test.each(["en", "ja"] as const)("renders Rust rows and dispatches the selected domainless room in %s", (locale) => {
  setActiveLocaleProfile(locale, "none");
  const onAdd = vi.fn();
  render(<AddExistingRoomDialog model={model()} spaceName="Synthetic Workspace" busy={false} onAdd={onAdd} onClose={vi.fn()} />);

  expect(screen.getByRole("dialog", { name: t("spaceAddRooms.title", { spaceName: "Synthetic Workspace" }) })).toBeTruthy();
  expect(rows()).toHaveLength(3);
  expect(within(rows()[2]).getByText(t("spaceAddRooms.added"))).toBeTruthy();
  expect(within(rows()[2]).queryByRole("button")).toBeNull();

  fireEvent.click(screen.getByRole("button", {
    name: t("spaceAddRooms.addAccessible", { roomName: "設計レビュー", spaceName: "Synthetic Workspace" })
  }));
  expect(onAdd).toHaveBeenCalledWith(DOMAINLESS);
  // A command receipt alone leaves the rows unchanged; Rust settles them.
  expect(within(rows()[1]).queryByText(t("spaceAddRooms.added"))).toBeNull();
});

test("search filters by name through the IME-safe field without reclassifying rows", () => {
  render(<AddExistingRoomDialog model={model()} spaceName="Synthetic Workspace" busy={false} onAdd={vi.fn()} onClose={vi.fn()} />);
  const search = screen.getByRole("searchbox", { name: t("spaceAddRooms.search") });
  fireEvent.change(search, { target: { value: "設計" } });
  expect(rows()).toHaveLength(1);
  expect(within(rows()[0]).getByText("設計レビュー")).toBeTruthy();
  fireEvent.change(search, { target: { value: "no such room" } });
  expect(screen.getByRole("status").textContent).toBe(t("spaceAddRooms.noMatches"));
});

test("pending rows disable every add action until Rust settles", () => {
  render(<AddExistingRoomDialog model={model({ domainless: { kind: "adding" } })} spaceName="Synthetic Workspace" busy={false} onAdd={vi.fn()} onClose={vi.fn()} />);
  expect(within(rows()[1]).getByText(t("spaceAddRooms.adding"))).toBeTruthy();
  const alphaAdd = screen.getByRole("button", {
    name: t("spaceAddRooms.addAccessible", { roomName: "Alpha notes", spaceName: "Synthetic Workspace" })
  }) as HTMLButtonElement;
  expect(alphaAdd.disabled).toBe(true);
});

test("failures are actionable and retry the same room", () => {
  const onAdd = vi.fn();
  render(
    <AddExistingRoomDialog
      model={model({ alpha: { kind: "failed", reason: "network" }, domainless: { kind: "failed", reason: "forbidden" } })}
      spaceName="Synthetic Workspace"
      busy={false}
      onAdd={onAdd}
      onClose={vi.fn()}
    />
  );
  expect(within(rows()[0]).getByRole("alert").textContent).toBe(
    t("spaceAddRooms.failed", { reason: operationFailureLabel("network") })
  );
  expect(within(rows()[1]).getByRole("alert").textContent).toBe(t("spaceAddRooms.failedForbidden"));
  fireEvent.click(screen.getByRole("button", { name: t("spaceAddRooms.retryAccessible", { roomName: "Alpha notes" }) }));
  expect(onAdd).toHaveBeenCalledWith("!alpha:example.invalid");
});

test("an empty projection explains that no joined rooms can be added", () => {
  render(<AddExistingRoomDialog model={{ space_id: "!space:example.invalid", candidates: [] }} spaceName="Synthetic Workspace" busy={false} onAdd={vi.fn()} onClose={vi.fn()} />);
  expect(screen.getByRole("status").textContent).toBe(t("spaceAddRooms.empty"));
});
