// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { setActiveLocaleProfile, t } from "../i18n/messages";
import { CreateEntityDialog } from "./dialogs";

afterEach(() => { cleanup(); setActiveLocaleProfile("en", "none"); });
test.each(["en", "ja"] as const)("renders Rust address preview and preserves drafts across visibility in %s", locale => {
  setActiveLocaleProfile(locale, "none");
  const onChange = vi.fn();
  render(<CreateEntityDialog kind="room" isBusy={false} value="設計"
    roomOptions={{ aliasLocalpart: "manual", topic: "", visibility: "public", encrypted: false }}
    addressPreview={{ localpart: "manual", full_alias: "#manual:example.invalid", error: null }}
    addressFailure="aliasInUse"
    onCancel={vi.fn()} onValueChange={vi.fn()} onSubmit={vi.fn()} onRoomOptionsChange={onChange} />);
  expect(screen.getByRole("alert").textContent).toBe(t("dialog.roomAddressInUse"));
  expect(screen.getByRole("link", { name: t("dialog.roomAddressAbout") }).getAttribute("href")).toBe("https://matrix.org/docs/chat_basics/public-rooms/");
  expect(screen.getByText(t("dialog.roomAddressHelp"))).toBeTruthy();
  expect(screen.getByText(t("dialog.roomAddressPreview", { address: "#manual:example.invalid" }))).toBeTruthy();
  expect(screen.getByLabelText(t("dialog.roomAddress")).getAttribute("aria-describedby")).toContain("create-room-address-help");
  fireEvent.click(screen.getByRole("radio", { name: t("dialog.privateRoom") }));
  expect(onChange).toHaveBeenLastCalledWith(expect.objectContaining({ visibility: "private", aliasLocalpart: "manual" }));
});
