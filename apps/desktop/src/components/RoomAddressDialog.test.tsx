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
  fireEvent.click(accessRadio("invite"));
  expect(onChange).toHaveBeenLastCalledWith(expect.objectContaining({ visibility: "private", aliasLocalpart: "manual" }));
});

/** Locale-independent selection among the create dialog's access choices. */
function accessRadio(value: string): HTMLElement {
  const radio = screen
    .getAllByRole("radio")
    .find((element) => (element as HTMLInputElement).value === value);
  if (!radio) throw new Error(`no access radio ${value}`);
  return radio;
}

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

// #1023: the room display name is optional; an unnamed room uses the
// SDK-calculated name. A Space still needs a name.
test.each(["en", "ja"] as const)("an unnamed room can be created, an unnamed Space cannot, in %s", locale => {
  setActiveLocaleProfile(locale, "none");
  const onSubmit = vi.fn();
  const view = render(<CreateEntityDialog kind="room" isBusy={false} value="  "
    roomOptions={{ aliasLocalpart: "", topic: "", visibility: "private", encrypted: true, invitedOnly: false }}
    onCancel={vi.fn()} onValueChange={vi.fn()} onSubmit={onSubmit} onRoomOptionsChange={vi.fn()} />);
  expect(screen.getByLabelText(t("dialog.roomName")).getAttribute("placeholder")).toBe(t("dialog.roomNameOptional"));
  expect(screen.getByText(t("dialog.roomNameOptionalHelp"))).toBeTruthy();
  const submit = screen.getByRole("button", { name: t("dialog.submitCreateRoom") }) as HTMLButtonElement;
  expect(submit.disabled).toBe(false);
  fireEvent.click(submit);
  expect(onSubmit).toHaveBeenCalledTimes(1);
  view.unmount();
  render(<CreateEntityDialog kind="space" isBusy={false} value=""
    onCancel={vi.fn()} onValueChange={vi.fn()} onSubmit={vi.fn()} />);
  expect((screen.getByRole("button", { name: t("dialog.submitCreateSpace") }) as HTMLButtonElement).disabled).toBe(true);
});

// #1023: an unnamed public room offers no address suggestion, so Rust reports
// that it will be created without one; that is not an error.
test.each(["en", "ja"] as const)("an unnamed public room may be created without an address in %s", locale => {
  setActiveLocaleProfile(locale, "none");
  render(<CreateEntityDialog kind="room" isBusy={false} value=""
    targetSpaceName="research-group"
    roomOptions={{ aliasLocalpart: "", topic: "", visibility: "public", encrypted: true, invitedOnly: false }}
    addressPreview={{ localpart: "", full_alias: null, error: null, server_name: "example.invalid", without_address: true }}
    onCancel={vi.fn()} onValueChange={vi.fn()} onSubmit={vi.fn()} onRoomOptionsChange={vi.fn()} />);
  expect(screen.getByRole("status").textContent).toBe(t("dialog.roomAddressNone"));
  expect(screen.getByLabelText(t("dialog.roomAddress")).getAttribute("aria-invalid")).toBe("false");
  expect((screen.getByRole("button", { name: t("dialog.submitCreateRoom") }) as HTMLButtonElement).disabled).toBe(false);
});

// #1023: switching to public no longer discards the private option's
// encryption choice; the request builder sends a public room unencrypted.
test("switching visibility keeps the private encryption choice", () => {
  const onChange = vi.fn();
  render(<CreateEntityDialog kind="room" isBusy={false} value=""
    roomOptions={{ aliasLocalpart: "", topic: "", visibility: "private", encrypted: true, invitedOnly: false }}
    onCancel={vi.fn()} onValueChange={vi.fn()} onSubmit={vi.fn()} onRoomOptionsChange={onChange} />);
  fireEvent.click(screen.getByRole("radio", { name: /Public/ }));
  expect(onChange).toHaveBeenLastCalledWith(expect.objectContaining({ visibility: "public", encrypted: true }));
});

test.each(["en", "ja"] as const)("an unnamed room's conflict does not quote an empty name in %s", locale => {
  setActiveLocaleProfile(locale, "none");
  render(<CreateEntityDialog kind="room" isBusy={false} value=""
    roomOptions={{ aliasLocalpart: "lobby", topic: "", visibility: "public", encrypted: true, invitedOnly: false }}
    addressPreview={{ localpart: "lobby", full_alias: "#lobby:example.invalid", error: null, server_name: "example.invalid" }}
    addressConflict={{ fullAddress: "#lobby:example.invalid", server: "example.invalid", roomName: "" }}
    onCancel={vi.fn()} onValueChange={vi.fn()} onSubmit={vi.fn()} onRoomOptionsChange={vi.fn()} />);
  expect(screen.getByRole("alert").textContent).toBe(t("dialog.roomAddressInUseUnnamed", {
    fullAddress: "#lobby:example.invalid", server: "example.invalid"
  }));
});

test("the unnamed conflict copy in English and Japanese", () => {
  const values = { fullAddress: "#lobby:example.invalid", server: "example.invalid" };
  expect(t("dialog.roomAddressInUseUnnamed", values)).toBe(
    "The address #lobby:example.invalid is already in use. Addresses are shared across all Spaces on example.invalid. Change the room address, for example by adding a project name or number, or clear it to create the room without an address."
  );
  setActiveLocaleProfile("ja", "none");
  expect(t("dialog.roomAddressInUseUnnamed", values)).toBe(
    "アドレス #lobby:example.invalid はすでに使われています。アドレスは example.invalid 上のすべての Space で共通です。ルームアドレスにプロジェクト名や数字などを追加するか、空欄にしてアドレスなしでルームを作成してください。"
  );
});
