// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CoreEventPayload, TimelineItem } from "../domain/coreEvents";
import type { OpenContextMenu } from "../app/uiShared";
import { setActiveLocaleProfile, t } from "../i18n/messages";
import { TimelineView, clearTimelineViewportSessionMemoryForTests } from "./TimelineView";
import { KEY, baseTransport, message } from "./timelineViewTestSupport";

afterEach(() => {
  cleanup();
  clearTimelineViewportSessionMemoryForTests();
  setActiveLocaleProfile("en", "none");
});

const OWN_USER = "@bob:example.invalid";

/** #2066: a message of the reader's own that Rust allows every write on, plus a
 * reaction someone else left, so each affordance is present when unlocked. */
function ownMessageWithReaction(): TimelineItem {
  return {
    ...message("$own:example.invalid", "Own message"),
    can_redact: true,
    actions: {
      ...message("$own:example.invalid", "").actions!,
      can_forward: true,
      can_copy: true,
      can_view_source: true
    },
    reactions: [
      {
        key: "👍",
        count: 1,
        reacted_by_me: false,
        my_reaction_event_id: null,
        sender_preview: [{ user_id: "@other-user:example.invalid", display_label: "Other User" }]
      }
    ]
  };
}

function renderRow(
  item: TimelineItem,
  canWrite: boolean,
  onOpenContextMenu: OpenContextMenu = vi.fn()
) {
  let emit: (payload: CoreEventPayload) => void = () => undefined;
  const sendReaction = vi.fn(async () => undefined);
  const transport = baseTransport({
    sendReaction,
    listenCoreEvents(listener) {
      emit = listener;
      return () => undefined;
    }
  });
  const view = (writable: boolean) => (
    <TimelineView
      timelineKey={KEY}
      roomId="!room:example.invalid"
      transport={transport}
      currentUserId={OWN_USER}
      canWrite={writable}
      onReply={vi.fn()}
      onOpenContextMenu={onOpenContextMenu}
    />
  );
  const rendered = render(view(canWrite));
  act(() =>
    emit({
      kind: "Timeline",
      event: { InitialItems: { request_id: null, key: KEY, generation: 1, items: [item] } }
    })
  );
  return {
    ...rendered,
    sendReaction,
    rerenderWritable: (writable: boolean) => rendered.rerender(view(writable))
  };
}

function rowOf(eventId: string): HTMLElement {
  const element = document.querySelector<HTMLElement>(`[data-event-id="${eventId}"]`);
  expect(element).toBeTruthy();
  return element!;
}

function reactionPill(): HTMLElement {
  return screen.getByRole("button", { name: /^Reaction 👍/ });
}

/** The repo has no jest-dom matchers, so assert the DOM property directly. */
function enabled(control: HTMLElement): boolean {
  return !(control as HTMLButtonElement).disabled;
}

/**
 * The items the row would hand the context menu. An own message with no
 * read-only entry opens no menu at all, which is why `opened` is separate from
 * the item ids.
 */
function contextMenu(onOpenContextMenu: OpenContextMenu): {
  opened: boolean;
  ids: string[];
} {
  const menu = vi.mocked(onOpenContextMenu);
  menu.mockClear();
  fireEvent.contextMenu(rowOf("$own:example.invalid"));
  const call = menu.mock.calls.at(-1);
  return { opened: call !== undefined, ids: (call?.[2] ?? []).map((item) => item.id) };
}

function openActionMenu(): void {
  fireEvent.click(screen.getByRole("button", { name: t("timeline.messageActions") }));
}

describe("TimelineView read-only write paths (#2066)", () => {
  it("offers all three paths while sending is admitted", () => {
    const onOpenContextMenu = vi.fn();
    renderRow(ownMessageWithReaction(), true, onOpenContextMenu);

    expect(enabled(reactionPill())).toBe(true);
    openActionMenu();
    expect(screen.getByRole("menuitem", { name: t("timeline.forwardMessage") })).toBeTruthy();
    const menu = contextMenu(onOpenContextMenu);
    expect(menu.opened).toBe(true);
    expect(menu.ids).toContain("redactMessage");
  });

  it("hides Forward from the message action menu while sending is locked", () => {
    renderRow(ownMessageWithReaction(), false);

    openActionMenu();
    expect(screen.queryByRole("menuitem", { name: t("timeline.forwardMessage") })).toBeNull();
    // The read-only entries stay: copy and source need no send admission.
    expect(screen.getByRole("menuitem", { name: t("timeline.copyMessage") })).toBeTruthy();
    expect(screen.getByRole("menuitem", { name: t("timeline.viewSource") })).toBeTruthy();
  });

  it("hides Redact from the context menu while sending is locked", () => {
    const onOpenContextMenu = vi.fn();
    renderRow(ownMessageWithReaction(), false, onOpenContextMenu);

    // Redact was this own message's only entry, so the menu itself is gone
    // rather than offered with nothing in it.
    const menu = contextMenu(onOpenContextMenu);
    expect(menu.opened).toBe(false);
    expect(menu.ids).not.toContain("redactMessage");
  });

  it("drops the message actions button when Forward was its only entry", () => {
    // No body and no permalink or source facts: unlocking offers `…` for
    // Forward alone, so the lock removes the button instead of emptying it.
    const bare = {
      ...message("$bare:example.invalid", ""),
      body: null,
      can_redact: true,
      actions: {
        ...message("$bare:example.invalid", "").actions!,
        can_forward: true
      }
    };
    const { rerenderWritable } = renderRow(bare, true);
    expect(screen.getByRole("button", { name: t("timeline.messageActions") })).toBeTruthy();

    rerenderWritable(false);
    expect(screen.queryByRole("button", { name: t("timeline.messageActions") })).toBeNull();
  });

  it("keeps the reaction count but takes no press while sending is locked", () => {
    const { sendReaction } = renderRow(ownMessageWithReaction(), false);
    const pill = reactionPill();

    expect(pill.textContent).toContain("1");
    expect(enabled(pill)).toBe(false);
    fireEvent.click(pill);
    expect(sendReaction).not.toHaveBeenCalled();
  });

  it("sends the reaction when the lock is not there", () => {
    const { sendReaction } = renderRow(ownMessageWithReaction(), true);

    fireEvent.click(reactionPill());

    expect(sendReaction).toHaveBeenCalledWith("!room:example.invalid", "$own:example.invalid", "👍");
  });

  it("restores every path when the lock lifts", () => {
    const onOpenContextMenu = vi.fn();
    const { rerenderWritable } = renderRow(ownMessageWithReaction(), false, onOpenContextMenu);
    expect(enabled(reactionPill())).toBe(false);

    rerenderWritable(true);

    expect(enabled(reactionPill())).toBe(true);
    openActionMenu();
    expect(screen.getByRole("menuitem", { name: t("timeline.forwardMessage") })).toBeTruthy();
    const menu = contextMenu(onOpenContextMenu);
    expect(menu.opened).toBe(true);
    expect(menu.ids).toContain("redactMessage");
  });
});
