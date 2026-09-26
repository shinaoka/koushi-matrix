// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { setActiveLocaleProfile, t } from "../i18n/messages";
import { CreateEntityDialog } from "./dialogs";
import type { RoomAddressAvailabilityState } from "../domain/types";

afterEach(() => { cleanup(); setActiveLocaleProfile("en", "none"); });
test.each(["en", "ja"] as const)("renders Rust address preview and preserves drafts across visibility in %s", locale => {
  setActiveLocaleProfile(locale, "none");
  const onChange = vi.fn();
  render(<CreateEntityDialog kind="room" isBusy={false} value="設計"
    roomOptions={{ aliasLocalpart: "manual", topic: "", visibility: "public", encrypted: false, invitedOnly: false }}
    addressPreview={{ localpart: "manual", full_alias: "#manual:example.invalid", error: null, server_name: "example.invalid" }}
    onCancel={vi.fn()} onValueChange={vi.fn()} onSubmit={vi.fn()} onRoomOptionsChange={onChange} />);
  expect(screen.queryByRole("alert")).toBeNull();
  expect(screen.getByRole("link", { name: t("dialog.roomAddressAbout") }).getAttribute("href")).toBe("https://matrix.org/docs/chat_basics/public-rooms/");
  expect(screen.getByText(`${t("dialog.roomAddressHelp")} ${t("dialog.roomAddressScope", { server: "example.invalid" })}`)).toBeTruthy();
  expect(screen.getByText(t("dialog.roomAddressPreview", { address: "#manual:example.invalid" }))).toBeTruthy();
  expect(screen.getByLabelText(t("dialog.roomAddress")).getAttribute("aria-describedby")).toContain("create-room-address-help");
  fireEvent.click(screen.getByRole("radio", { name: t("dialog.privateRoom") }));
  expect(onChange).toHaveBeenLastCalledWith(expect.objectContaining({ visibility: "private", aliasLocalpart: "manual" }));
});

// #1006: an alias conflict names the attempted address and its server-wide
// scope, keeps the room name, and puts the user at the address field.
test.each(["en", "ja"] as const)("an alias conflict is actionable at the address field in %s", locale => {
  setActiveLocaleProfile(locale, "none");
  render(<CreateEntityDialog kind="room" isBusy={false} value="papers"
    targetSpaceName="research-group"
    roomOptions={{ aliasLocalpart: "research-group-papers", topic: "", visibility: "public", encrypted: false, invitedOnly: false }}
    addressPreview={{ localpart: "research-group-papers", full_alias: "#research-group-papers:example.invalid", error: null, server_name: "example.invalid" }}
    addressConflict={{ fullAddress: "#research-group-papers:example.invalid", server: "example.invalid", roomName: "papers" }}
    onCancel={vi.fn()} onValueChange={vi.fn()} onSubmit={vi.fn()} onRoomOptionsChange={vi.fn()} />);
  const alert = screen.getByRole("alert");
  expect(alert.textContent).toBe(t("dialog.roomAddressInUse", {
    fullAddress: "#research-group-papers:example.invalid", server: "example.invalid", roomName: "papers"
  }));
  expect(alert.textContent).toContain("#research-group-papers:example.invalid");
  expect(alert.textContent).toContain("example.invalid");
  expect(alert.textContent).toContain("papers");
  const address = screen.getByLabelText(t("dialog.roomAddress"));
  expect(document.activeElement).toBe(address);
  expect(address.getAttribute("aria-invalid")).toBe("true");
  expect(address.getAttribute("aria-describedby")).toContain("create-room-address-conflict");
  // The display name draft and the target Space stay in place.
  expect((screen.getByLabelText(t("dialog.roomName")) as HTMLInputElement).value).toBe("papers");
  expect(screen.getByText(t("dialog.publicRoomInSpace", { spaceName: "research-group" }))).toBeTruthy();
});

test("the English and Japanese conflict copy follows the issue wording", () => {
  const values = { fullAddress: "#papers:example.invalid", server: "example.invalid", roomName: "papers" };
  expect(t("dialog.roomAddressInUse", values)).toBe(
    "The address #papers:example.invalid is already in use. Addresses are shared across all Spaces on example.invalid. You can keep the room name ‘papers’; change only the room address, for example by adding a project name or number."
  );
  setActiveLocaleProfile("ja", "none");
  expect(t("dialog.roomAddressInUse", values)).toBe(
    "アドレス #papers:example.invalid はすでに使われています。アドレスは example.invalid 上のすべての Space で共通です。ルーム名『papers』はそのままで、ルームアドレスにプロジェクト名や数字などを追加してください。"
  );
});

test("a public room at Home shows no Space note", () => {
  render(<CreateEntityDialog kind="room" isBusy={false} value="papers"
    roomOptions={{ aliasLocalpart: "papers", topic: "", visibility: "public", encrypted: false, invitedOnly: false }}
    addressPreview={{ localpart: "papers", full_alias: "#papers:example.invalid", error: null, server_name: "example.invalid" }}
    onCancel={vi.fn()} onValueChange={vi.fn()} onSubmit={vi.fn()} onRoomOptionsChange={vi.fn()} />);
  expect(screen.queryByText(t("dialog.publicRoomInSpace", { spaceName: "research-group" }))).toBeNull();
});

// #1006: the advisory availability note renders only Rust-owned results and
// offers an unchecked alternative behind an explicit action.
function renderWithAvailability(
  availability: RoomAddressAvailabilityState,
  onUse = vi.fn(),
  conflict = false
) {
  render(<CreateEntityDialog kind="room" isBusy={false} value="papers"
    roomOptions={{ aliasLocalpart: "papers", topic: "", visibility: "public", encrypted: false, invitedOnly: false }}
    addressPreview={{ localpart: "papers", full_alias: "#papers:example.invalid", error: null, server_name: "example.invalid" }}
    addressConflict={conflict ? { fullAddress: "#papers:example.invalid", server: "example.invalid", roomName: "papers" } : null}
    addressAvailability={availability}
    onUseSuggestedAddress={onUse}
    onCancel={vi.fn()} onValueChange={vi.fn()} onSubmit={vi.fn()} onRoomOptionsChange={vi.fn()} />);
  return onUse;
}

test.each(["en", "ja"] as const)("advisory results are labeled as not reserving the address in %s", locale => {
  setActiveLocaleProfile(locale, "none");
  renderWithAvailability({ kind: "checking", request_id: 1, full_alias: "#papers:example.invalid" });
  expect(screen.getByText(t("dialog.roomAddressChecking", { address: "#papers:example.invalid" }))).toBeTruthy();
  cleanup();
  renderWithAvailability({ kind: "checked", request_id: 1, full_alias: "#papers:example.invalid", availability: "available", suggestion: null });
  expect(screen.getByText(t("dialog.roomAddressAvailable", { address: "#papers:example.invalid" }))).toBeTruthy();
  cleanup();
  renderWithAvailability({ kind: "checked", request_id: 1, full_alias: "#papers:example.invalid", availability: "unknown", suggestion: null });
  expect(screen.getByText(t("dialog.roomAddressCheckUnknown"))).toBeTruthy();
  // Submission is never blocked by an advisory result.
  expect((screen.getByRole("button", { name: t("dialog.submitCreateRoom") }) as HTMLButtonElement).disabled).toBe(false);
});

test("an address in use offers a labeled suggestion that is used only on request", () => {
  const onUse = renderWithAvailability({
    kind: "checked", request_id: 2, full_alias: "#papers:example.invalid", availability: "inUse",
    suggestion: { localpart: "papers-2", full_alias: "#papers-2:example.invalid" }
  });
  expect(screen.getByText(t("dialog.roomAddressTaken", { address: "#papers:example.invalid", server: "example.invalid" }))).toBeTruthy();
  expect(screen.getByText(t("dialog.roomAddressSuggestion", { address: "#papers-2:example.invalid" }))).toBeTruthy();
  expect(onUse).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: t("dialog.roomAddressUseSuggestion") }));
  expect(onUse).toHaveBeenCalledWith("papers-2");
});

test("while the authoritative conflict is shown only the suggestion is added", () => {
  renderWithAvailability({
    kind: "checked", request_id: 2, full_alias: "#papers:example.invalid", availability: "available", suggestion: null
  }, vi.fn(), true);
  expect(screen.queryByText(t("dialog.roomAddressAvailable", { address: "#papers:example.invalid" }))).toBeNull();
  expect(screen.getByRole("alert")).toBeTruthy();
});
