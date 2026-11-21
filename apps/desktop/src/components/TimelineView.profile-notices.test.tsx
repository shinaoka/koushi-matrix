// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CoreEventPayload, TimelineNoticeI18nKey } from "../domain/coreEvents";
import { setActiveLocaleProfile, t } from "../i18n/messages";
import { TimelineView, clearTimelineViewportSessionMemoryForTests } from "./TimelineView";
import { KEY, baseTransport, message } from "./timelineViewTestSupport";
import type { TimelineItem } from "../domain/coreEvents";

afterEach(() => {
  cleanup();
  clearTimelineViewportSessionMemoryForTests();
  setActiveLocaleProfile("en", "none");
});

function profileNotices(): TimelineItem[] {
  const cases: [TimelineNoticeI18nKey, number][] = [
    ["timeline.notice.profileDisplayNameChanged", 0],
    ["timeline.notice.profileAvatarChanged", 1],
    ["timeline.notice.profileDisplayNameAndAvatarChanged", 2],
    ["timeline.notice.profileUpdated", 3]
  ];
  return cases.map(([key, index]) => ({
    ...message(`$profile-${index}`, "changed their display name"),
    sender_label: "Member 1",
    message_kind: "notice" as const,
    notice_i18n: { key }
  }));
}

/** #2041: every action a notice row must not offer is present on the fixture,
 * so their absence comes from the row's own rule rather than from Rust saying no. */
function noticeWithEveryAction(eventId = "$notice"): TimelineItem {
  return {
    ...message(eventId, "set the room topic to Topic"),
    sender_label: "Member 1",
    message_kind: "notice",
    notice_i18n: { key: "timeline.notice.roomTopicSet", value: "Topic" },
    can_edit: true,
    can_redact: true,
    actions: {
      ...message(eventId, "").actions!,
      can_forward: true,
      can_copy: true,
      can_reply: true,
      can_permalink: true,
      can_view_source: true
    }
  };
}

function renderTimeline(
  items: TimelineItem[],
  handlers: {
    onOpenSenderProfile?: (roomId: string, userId: string) => void;
    onOpenContextMenu?: (...args: never[]) => void;
    pinnedEventIds?: string[];
  } = {}
) {
  let emit: (payload: CoreEventPayload) => void = () => undefined;
  const transport = baseTransport({
    listenCoreEvents(listener) {
      emit = listener;
      return () => undefined;
    }
  });
  render(
    <TimelineView
      timelineKey={KEY}
      roomId="!room:example.invalid"
      transport={transport}
      onReply={vi.fn()}
      onOpenSenderProfile={handlers.onOpenSenderProfile}
      onOpenContextMenu={handlers.onOpenContextMenu as never}
      pinnedEventIds={handlers.pinnedEventIds}
    />
  );
  act(() =>
    emit({
      kind: "Timeline",
      event: {
        InitialItems: { request_id: null, key: KEY, generation: 1, items }
      }
    })
  );
}

function rowOf(eventId: string): HTMLElement {
  const row = document.querySelector<HTMLElement>(`[data-event-id="${eventId}"]`);
  expect(row).toBeTruthy();
  return row!;
}

describe("TimelineView profile notices (#2041)", () => {
  it.each(["en", "ja"] as const)(
    "identifies the member in localized profile notices in %s, even without a sender header",
    async (locale) => {
      setActiveLocaleProfile(locale, "none");
      renderTimeline([message("$before-profile", "Synthetic message"), ...profileNotices()]);

      const cases: [string, string][] = [
        ["$profile-0", "Member 1 changed their display name"],
        ["$profile-1", "Member 1 changed their profile picture"],
        ["$profile-2", "Member 1 changed their display name and profile picture"],
        ["$profile-3", "Member 1 updated their room profile"]
      ];
      const ja: Record<string, string> = {
        "$profile-0": "Member 1が表示名を変更しました",
        "$profile-1": "Member 1がプロフィール画像を変更しました",
        "$profile-2": "Member 1が表示名とプロフィール画像を変更しました",
        "$profile-3": "Member 1がルーム内のプロフィールを更新しました"
      };
      await screen.findByText("Synthetic message");
      for (const [eventId, en] of cases) {
        const row = rowOf(eventId);
        // #2041: the sender name is its own element so it can open a profile,
        // so the sentence is read off the row, not off a single text node.
        expect(row.textContent?.replace(/\s+/g, " ").trim()).toBe(locale === "ja" ? ja[eventId] : en);
      }
      // The localized sentence wins over the fallback body, which the fixture
      // sets to the same words, so the exact-line assertion above is the check.
      expect(rowOf("$profile-0").classList.contains("is-continuation")).toBe(true);
      expect(rowOf("$profile-0").querySelector(".message-notice-controls")).not.toBeNull();
    }
  );

  it.each(["en", "ja", "ko", "zh-Hans", "zh-Hant"] as const)(
    "keeps the sender first in every profile notice sentence in %s",
    (locale) => {
      // The line drops this prefix and shows the name itself, so every catalog
      // has to start the sentence with it. A translation that reorders the name
      // would otherwise render it twice.
      setActiveLocaleProfile(locale, "none");
      for (const key of [
        "timeline.notice.profileDisplayNameChanged",
        "timeline.notice.profileAvatarChanged",
        "timeline.notice.profileDisplayNameAndAvatarChanged",
        "timeline.notice.profileUpdated"
      ] as const) {
        expect(t(key, { sender: "Probe Name" }).startsWith("Probe Name")).toBe(true);
      }
    }
  );

  it("draws a notice as one muted information line with no avatar and no message actions", async () => {
    renderTimeline([noticeWithEveryAction(), message("$after", "An ordinary message")]);
    await screen.findByText("An ordinary message");

    const row = rowOf("$notice");
    expect(row.classList.contains("message-notice-row")).toBe(true);
    expect(row.dataset.messageKind).toBe("notice");
    // No avatar slot, while the following message row still has one.
    expect(row.querySelector(".avatar")).toBeNull();
    expect(rowOf("$after").querySelector(".avatar")).not.toBeNull();

    // No hover action bar at all: the row's two controls sit on the line.
    expect(row.querySelector(".message-actions")).toBeNull();
    expect(row.querySelector(".message-notice-controls")).not.toBeNull();

    const scoped = within(row);
    // Reply and Edit are message actions; a state event has neither.
    expect(scoped.queryByRole("button", { name: "Reply to message" })).toBeNull();
    expect(scoped.queryByRole("button", { name: "Edit message" })).toBeNull();
    expect(scoped.queryByRole("button", { name: "Redact message" })).toBeNull();
    expect(scoped.queryByRole("button", { name: "Reply in thread" })).toBeNull();
    // The affordances a notice keeps: react and pin.
    expect(scoped.getByRole("button", { name: "Add reaction" })).toBeTruthy();
    expect(scoped.getByRole("button", { name: "Pin message" })).toBeTruthy();
    // #2041: the read-only menu moves onto the line, so Copy and View source
    // stay reachable without a bar; a state event has no Forward to offer.
    fireEvent.click(scoped.getByRole("button", { name: "Message actions" }));
    const menu = scoped.getByRole("menu", { name: "Message actions" });
    expect(within(menu).getByRole("menuitem", { name: "Copy message" })).toBeTruthy();
    expect(within(menu).getByRole("menuitem", { name: "View source" })).toBeTruthy();
    expect(within(menu).queryByRole("menuitem", { name: "Forward message" })).toBeNull();
    expect(within(menu).queryByRole("menuitem", { name: "Edit message" })).toBeNull();
    expect(rowOf("$after").querySelector(".message-actions")).not.toBeNull();
  });

  it("keeps a pinned notice's Unpin and its existing reaction pills", async () => {
    const pinned: TimelineItem = {
      ...noticeWithEveryAction("$pinned-notice"),
      reactions: [
        { key: "👍", count: 2, reacted_by_me: false, my_reaction_event_id: null, sender_preview: [] }
      ]
    };
    // Rust owns pinnability (#1704); a pinned notice still offers Unpin.
    renderTimeline([pinned], { pinnedEventIds: ["$pinned-notice"] });
    await screen.findByText(/set the room topic to Topic/);
    const scoped = within(rowOf("$pinned-notice"));
    expect(scoped.getByRole("button", { name: "Reaction 👍, count 2" })).toBeTruthy();
    expect(scoped.getByRole("button", { name: "Unpin message" })).toBeTruthy();
    expect(scoped.queryByRole("button", { name: "Pin message" })).toBeNull();
  });

  it("separates the sender from a sentence that does not name it", async () => {
    // A topic notice's localized sentence carries no sender, so the line has to
    // supply the space between the name and the sentence (#2041 review).
    renderTimeline([noticeWithEveryAction()]);
    await screen.findByText(/set the room topic to Topic/);
    expect(rowOf("$notice").textContent?.replace(/\s+/g, " ").trim()).toBe(
      "Member 1 set the room topic to Topic"
    );
  });

  it("opens the sender's profile from the notice line", async () => {
    const onOpenSenderProfile = vi.fn();
    renderTimeline([noticeWithEveryAction()], { onOpenSenderProfile });
    await screen.findByText(/set the room topic to Topic/);

    const sender = within(rowOf("$notice")).getByRole("button", {
      name: "Open profile for Member 1"
    });
    fireEvent.click(sender);
    expect(onOpenSenderProfile).toHaveBeenCalledTimes(1);
    expect(onOpenSenderProfile).toHaveBeenCalledWith(
      "!room:example.invalid",
      "@bob:example.invalid"
    );
  });

  it("keeps the notice text selectable and offers Copy on the selection", async () => {
    const onOpenContextMenu = vi.fn();
    renderTimeline([noticeWithEveryAction()], { onOpenContextMenu });
    await screen.findByText(/set the room topic to Topic/);

    const row = rowOf("$notice");
    const walker = document.createTreeWalker(row, NodeFilter.SHOW_TEXT);
    let bodyText: Text | null = null;
    while (walker.nextNode()) {
      const candidate = walker.currentNode as Text;
      if (candidate.data.includes("set the room topic")) {
        bodyText = candidate;
        break;
      }
    }
    expect(bodyText).toBeTruthy();
    const range = document.createRange();
    range.setStart(bodyText!, 0);
    range.setEnd(bodyText!, "set the".length);
    const selection = window.getSelection();
    selection?.removeAllRanges();
    selection?.addRange(range);

    fireEvent.contextMenu(row);

    expect(onOpenContextMenu).toHaveBeenCalledTimes(1);
    const [, target, items] = onOpenContextMenu.mock.calls[0] as [
      unknown,
      { selectedText?: string },
      { id: string }[]
    ];
    expect(target).toMatchObject({ kind: "message", selectedText: "set the" });
    expect(items.map((item) => item.id)).toEqual(
      expect.arrayContaining(["copySelectedText", "searchWebForSelectedText"])
    );
    // Reply and Edit stay off the notice's context menu too.
    expect(items.map((item) => item.id)).not.toEqual(
      expect.arrayContaining(["replyToMessage", "editMessage"])
    );
  });

  it("keeps the notice row free of the message body box and the reply affordance", async () => {
    renderTimeline([noticeWithEveryAction()]);
    const row = rowOf("$notice");
    const body = row.querySelector(".message-body");
    expect(body?.classList.contains("message-notice")).toBe(true);
    // The notice body is not a reply target and shows no reply quote.
    expect(row.dataset.reply).toBeUndefined();
    expect(row.querySelector(".reply-quote")).toBeNull();
  });
});
