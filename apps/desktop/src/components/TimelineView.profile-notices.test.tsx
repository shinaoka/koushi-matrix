// @vitest-environment jsdom
import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { CoreEventPayload, TimelineNoticeI18nKey } from "../domain/coreEvents";
import { setActiveLocaleProfile } from "../i18n/messages";
import { TimelineView, clearTimelineViewportSessionMemoryForTests } from "./TimelineView";
import { KEY, baseTransport, message } from "./timelineViewTestSupport";

afterEach(() => {
  cleanup();
  clearTimelineViewportSessionMemoryForTests();
  setActiveLocaleProfile("en", "none");
});

it.each(["en", "ja"] as const)("identifies the member in localized profile notices in %s, even without a sender header", async (locale) => {
  setActiveLocaleProfile(locale, "none");
  let emit: (payload: CoreEventPayload) => void = () => undefined;
  const transport = baseTransport({
    listenCoreEvents(listener) { emit = listener; return () => undefined; }
  });
  render(<TimelineView timelineKey={KEY} roomId="!room:example.invalid" transport={transport} onReply={vi.fn()} />);
  const cases: [TimelineNoticeI18nKey, string, string][] = [
    ["timeline.notice.profileDisplayNameChanged", "Member 1 changed their display name", "Member 1が表示名を変更しました"],
    ["timeline.notice.profileAvatarChanged", "Member 1 changed their profile picture", "Member 1がプロフィール画像を変更しました"],
    ["timeline.notice.profileDisplayNameAndAvatarChanged", "Member 1 changed their display name and profile picture", "Member 1が表示名とプロフィール画像を変更しました"],
    ["timeline.notice.profileUpdated", "Member 1 updated their room profile", "Member 1がルーム内のプロフィールを更新しました"]
  ];
  const items = cases.map(([key], index) => ({
    ...message(`$profile-${index}`, "changed their display name"),
    sender_label: "Member 1",
    message_kind: "notice" as const,
    notice_i18n: { key }
  }));
  act(() => emit({ kind: "Timeline", event: { InitialItems: { request_id: null, key: KEY, generation: 1, items: [message("$before-profile", "Synthetic message"), ...items] } } }));
  for (const [, en, ja] of cases) {
    expect(await screen.findByText(locale === "ja" ? ja : en)).toBeTruthy();
  }
  expect(screen.queryByText("changed their display name")).toBeNull();
  expect(document.querySelector('[data-event-id="$profile-0"]')?.classList.contains("is-continuation")).toBe(true);
});
