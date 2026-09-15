// @vitest-environment jsdom
import { render, screen, cleanup } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { TopBar } from "./Shell";
import { contextMenuItems } from "../domain/contextMenus";
import { shortcutActionFromMenuPayload, shortcutById } from "../domain/shortcuts";
afterEach(cleanup);

test("help is not a question-mark shortcut button or a separate keyboard destination", () => {
  render(<TopBar activeSpaceName="Example" isBusy={false} searchInputRef={{ current: null }} searchQuery="" searchScope="allRooms" sync="running" onRestartSync={vi.fn()} onSearchQueryChange={vi.fn()} onSearchScopeChange={vi.fn()} />);
  expect(screen.queryByRole("button", { name: "Keyboard settings" })).toBeNull();
  expect(shortcutById("showKeyboardSettings")).toBeUndefined();
  expect(contextMenuItems({ kind: "account" }).some((item) => item.id === "openKeyboardSettings")).toBe(false);
  expect(shortcutActionFromMenuPayload("showHelp")).toBe("showHelp");
});
