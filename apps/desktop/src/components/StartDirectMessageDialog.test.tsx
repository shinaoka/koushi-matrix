// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";
import type { TimelineItem } from "../domain/coreEvents";
import { t } from "../i18n/messages";
import { StartDirectMessageDialog } from "./StartDirectMessageDialog";
import { TimelineItemRow } from "./timeline/TimelineItemRow";

afterEach(() => {
  cleanup();
});

describe("StartDirectMessageDialog (#1702)", () => {
  function renderDialog(isBusy = false) {
    const onCancel = vi.fn();
    const onConfirm = vi.fn();
    render(
      <StartDirectMessageDialog
        name="Grace Hopper"
        userId="@grace:example.invalid"
        isBusy={isBusy}
        onCancel={onCancel}
        onConfirm={onConfirm}
      />
    );
    return { onCancel, onConfirm };
  }

  test("names the person and the invite, with Cancel as the default focus", () => {
    renderDialog();
    const dialog = screen.getByRole("dialog", { name: t("room.startDirectMessageConfirmTitle") });
    expect(dialog.textContent).toContain(
      t("room.startDirectMessageConfirmBody", {
        name: "Grace Hopper",
        userId: "@grace:example.invalid"
      })
    );
    expect(document.activeElement).toBe(screen.getByRole("button", { name: t("action.cancel") }));
  });

  test("Cancel only dismisses; Start chat is the one action that confirms", () => {
    const { onCancel, onConfirm } = renderDialog();
    fireEvent.click(screen.getByRole("button", { name: t("action.cancel") }));
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(onConfirm).not.toHaveBeenCalled();

    fireEvent.click(
      screen.getByRole("button", { name: t("room.startDirectMessageConfirmAction") })
    );
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });

  test("both actions are disabled while the start is in flight", () => {
    renderDialog(true);
    const cancel = screen.getByRole<HTMLButtonElement>("button", { name: t("action.cancel") });
    const confirm = screen.getByRole<HTMLButtonElement>("button", {
      name: t("room.startDirectMessageConfirmAction")
    });
    expect(cancel.disabled).toBe(true);
    expect(confirm.disabled).toBe(true);
  });
});

test("a sender avatar passes the sender and its Rust display label to the start", () => {
  const onStartDirectMessage = vi.fn();
  render(
    <TimelineItemRow
      item={
        {
          id: { Event: { event_id: "$event:example.invalid" } },
          sender: "@grace:example.invalid",
          sender_label: "Grace Hopper",
          body: "hello",
          timestamp_ms: 1_820_000_000_000,
          in_reply_to_event_id: null,
          thread_root: null,
          can_react: true,
          is_redacted: false,
          is_hidden: false,
          can_redact: false,
          is_edited: false,
          can_edit: false,
          reactions: []
        } as unknown as TimelineItem
      }
      roomId="!room:example.invalid"
      currentUserId="@me:example.invalid"
      onStartDirectMessage={onStartDirectMessage}
      onReply={() => undefined}
      onSendReaction={() => undefined}
      onRedactReaction={() => undefined}
      onEdit={() => undefined}
      onRedact={() => undefined}
    />
  );

  fireEvent.click(
    screen.getByRole("button", { name: t("room.messageMember", { name: "Grace Hopper" }) })
  );

  expect(onStartDirectMessage).toHaveBeenCalledWith("@grace:example.invalid", "Grace Hopper");
});
