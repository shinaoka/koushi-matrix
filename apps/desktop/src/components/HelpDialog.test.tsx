// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { HelpDialog, HELP_REPOSITORY_URL } from "./HelpDialog";
import { setActiveLocaleProfile } from "../i18n/messages";

afterEach(() => { cleanup(); vi.unstubAllGlobals(); setActiveLocaleProfile("en"); });

test("help copies only the public repository URL and confirms success", async () => {
  const writeText = vi.fn().mockResolvedValue(undefined);
  vi.stubGlobal("navigator", { clipboard: { writeText } });
  render(<HelpDialog onClose={vi.fn()} />);
  expect(screen.getByRole("dialog", { name: "Koushi Help" }).hasAttribute("open")).toBe(true);
  expect(screen.getByText(/ChatGPT/)).toBeTruthy();
  expect(screen.queryByText("Composer send shortcut")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Copy GitHub URL" }));
  await waitFor(() => expect(screen.getByRole("status").textContent).toBe("URL copied"));
  expect(writeText).toHaveBeenCalledExactlyOnceWith(HELP_REPOSITORY_URL);
});

test.each(["unavailable", "denied"])("help leaves a selectable URL and reports clipboard %s", async (kind) => {
  vi.stubGlobal("navigator", kind === "unavailable" ? {} : { clipboard: { writeText: vi.fn().mockRejectedValue(new Error("denied")) } });
  render(<HelpDialog onClose={vi.fn()} />);
  fireEvent.click(screen.getByRole("button", { name: "Copy GitHub URL" }));
  await waitFor(() => expect(screen.getByRole("status").textContent).toMatch(/Select and copy/));
  expect(screen.getByText(HELP_REPOSITORY_URL)).toBeTruthy();
});

test("Japanese help exposes the same URL and a working close action", () => {
  setActiveLocaleProfile("ja");
  const close = vi.fn();
  render(<HelpDialog onClose={close} />);
  expect(screen.getByRole("dialog", { name: "Koushiのヘルプ" })).toBeTruthy();
  expect(screen.getByRole("button", { name: "GitHub URLをコピー" })).toBeTruthy();
  fireEvent(screen.getByRole("dialog"), new Event("cancel", { bubbles: true, cancelable: true }));
  expect(close).toHaveBeenCalledOnce();
});
