// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";

import { setActiveLocaleProfile } from "../i18n/messages";
import { applyTimelineEvent, createTimelineStore } from "../domain/timelineStore";
import { TimelineStoreContext } from "./timelineStoreContext";
import { TimelineView } from "./TimelineView";
import { baseTransport, message, KEY } from "./timelineViewTestSupport";

afterEach(() => {
  cleanup();
  setActiveLocaleProfile("en", "none");
});

describe("TimelineView sender avatars", () => {
  test("messages a non-self sender from an accessible avatar button, but own avatar is inert", () => {
    const onStartDirectMessage = vi.fn();
    const onOpenSenderProfile = vi.fn();
    const store = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [
          {
            ...message("$other", "From another person"),
            sender: "@other:example.invalid",
            sender_label: "Other Person"
          },
          {
            ...message("$own", "From me"),
            sender: "@me:example.invalid",
            sender_label: "Me"
          }
        ]
      }
    });

    render(
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={baseTransport({})}
          onReply={vi.fn()}
          onOpenSenderProfile={onOpenSenderProfile}
          onStartDirectMessage={onStartDirectMessage}
          currentUserId="@me:example.invalid"
        />
      </TimelineStoreContext.Provider>
    );

    fireEvent.click(screen.getByRole("button", { name: "Message Other Person" }));
    expect(onStartDirectMessage).toHaveBeenCalledWith("@other:example.invalid");
    expect(screen.queryByRole("button", { name: "Message Me" })).toBeNull();
    expect(onOpenSenderProfile).not.toHaveBeenCalled();
  });

  test("keeps continuation sender profiles interactive outside compact density and matches typography", () => {
    const onOpenSenderProfile = vi.fn();
    const store = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [
          { ...message("$one", "First"), sender: "@other:example.invalid", sender_label: "Other Person" },
          { ...message("$two", "Second"), sender: "@other:example.invalid", sender_label: "Other Person" }
        ]
      }
    });

    const { rerender } = render(
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={baseTransport({})}
          onReply={vi.fn()}
          onOpenSenderProfile={onOpenSenderProfile}
          density="default"
        />
      </TimelineStoreContext.Provider>
    );

    const normalProfileButtons = screen.getAllByRole("button", {
      name: "Open profile for Other Person"
    });
    expect(normalProfileButtons).toHaveLength(2);
    expect(normalProfileButtons[0].classList.contains("sender-profile-button")).toBe(true);
    expect(normalProfileButtons[1].classList.contains("sender-profile-button")).toBe(true);
    fireEvent.click(normalProfileButtons[1]);
    expect(onOpenSenderProfile).toHaveBeenCalledWith("!room:example.invalid", "@other:example.invalid");

    rerender(
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={baseTransport({})}
          onReply={vi.fn()}
          onOpenSenderProfile={onOpenSenderProfile}
          density="compact"
        />
      </TimelineStoreContext.Provider>
    );
    expect(screen.getAllByRole("button", { name: "Open profile for Other Person" })).toHaveLength(1);
    expect(screen.getAllByText("Other Person", { selector: "span.sender" })).toHaveLength(1);
  });
});
