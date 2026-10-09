// @vitest-environment jsdom
import { fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";

import { t } from "../i18n/messages";
import { AccessChoiceDetail, outcomeLineText } from "./AccessChoiceDetail";

afterEach(() => {
  document.body.innerHTML = "";
});

const choices = [
  { value: "public", label: "Public", summary: "Anyone can join." },
  { value: "invite", label: "Invite only", summary: "Only invited people can join." },
  {
    value: "restricted",
    label: "Members of a Space",
    summary: "Members can join.",
    disabled: true,
    disabledReason: "This room's conditions cannot be rewritten."
  }
];

function renderEditor(overrides: Partial<Parameters<typeof AccessChoiceDetail>[0]> = {}) {
  const props = {
    property: "join-rule",
    label: "Join rule",
    choices,
    selected: "public",
    details: <p>Rust details</p>,
    detailsConfirmed: false,
    canEdit: true,
    saveEnabled: true,
    saveLabel: "Save join rule",
    onSelect: vi.fn(),
    onSave: vi.fn(),
    onCancel: vi.fn(),
    ...overrides
  };
  return { ...render(<AccessChoiceDetail {...props} />), props };
}

test("lists every choice, marks the selection and states a disabled reason", () => {
  renderEditor();
  const group = screen.getByRole("radiogroup", { name: t("room.accessChooseLabel") });
  expect(within(group).getAllByRole("radio")).toHaveLength(3);
  expect((within(group).getByRole("radio", { name: /Public/ }) as HTMLInputElement).checked).toBe(
    true
  );
  const disabled = within(group).getByRole("radio", {
    name: /Members of a Space/
  }) as HTMLInputElement;
  expect(disabled.disabled).toBe(true);
  expect(screen.getByText("This room's conditions cannot be rewritten.")).toBeTruthy();
  // The details state is explicitly unsaved while the draft differs.
  expect(screen.getByText(t("room.accessDetailsUnsaved"))).toBeTruthy();
});

test("reports an enabled selection to the Rust draft", () => {
  const onSelect = vi.fn();
  renderEditor({ selected: "public", onSelect });
  fireEvent.click(screen.getByRole("radio", { name: /Invite only/ }));
  expect(onSelect).toHaveBeenCalledWith("invite");
});

test("Save is disabled until the caller enables it, and never while a rejection stands", () => {
  const { rerender, props } = renderEditor({ saveEnabled: false });
  expect(
    (screen.getByRole("button", { name: "Save join rule" }) as HTMLButtonElement).disabled
  ).toBe(true);
  rerender(<AccessChoiceDetail {...props} saveEnabled />);
  expect(
    (screen.getByRole("button", { name: "Save join rule" }) as HTMLButtonElement).disabled
  ).toBe(false);
  rerender(<AccessChoiceDetail {...props} rejection="A public room cannot be restricted." />);
  expect(
    (screen.getByRole("button", { name: "Save join rule" }) as HTMLButtonElement).disabled
  ).toBe(true);
  expect(screen.getByText("A public room cannot be restricted.")).toBeTruthy();
});

test("Escape cancels and returns focus to the card heading", () => {
  const onCancel = vi.fn();
  renderEditor({ onCancel });
  const group = screen.getByRole("group", { name: t("room.accessChooseLabel") });
  fireEvent.keyDown(group, { key: "Escape" });
  expect(onCancel).toHaveBeenCalledTimes(1);
  expect(document.activeElement).toBe(screen.getByRole("heading", { level: 4 }));
});

test("every choice and both actions are reachable in document order", () => {
  renderEditor();
  const group = screen.getByRole("radiogroup", { name: t("room.accessChooseLabel") });
  const radios = within(group).getAllByRole("radio");
  const save = screen.getByRole("button", { name: "Save join rule" });
  const cancel = screen.getByRole("button", { name: "Cancel" });
  for (const radio of radios) {
    expect(radio.compareDocumentPosition(save) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  }
  expect(save.compareDocumentPosition(cancel) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
});

test("a read-only editor shows the reason and no actions", () => {
  renderEditor({ canEdit: false, readOnlyReason: "You cannot change this.", onSave: undefined });
  expect(screen.getByText("You cannot change this.")).toBeTruthy();
  expect(screen.queryByRole("button")).toBeNull();
});

test("an outcome line substitutes the resolved Space name", () => {
  expect(
    outcomeLineText({ messageId: "room.accessOutcomeJoinSpaceMembers", substitutions: ["Design"] })
  ).toBe(t("room.accessOutcomeJoinSpaceMembers", { space: "Design" }));
});
