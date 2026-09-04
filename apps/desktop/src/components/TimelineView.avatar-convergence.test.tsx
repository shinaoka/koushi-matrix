// @vitest-environment jsdom

import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";

import type { CoreEventPayload } from "../domain/coreEvents";
import type { AvatarImage, AvatarThumbnailState, LiveSignalsState } from "../domain/types";
import { applyTimelineEvent, createTimelineStore } from "../domain/timelineStore";
import { setActiveLocaleProfile } from "../i18n/messages";
import { TimelineView } from "./TimelineView";
import { TimelineStoreContext } from "./timelineStoreContext";
import { baseTransport, message, KEY } from "./timelineViewTestSupport";

const AVATAR_URI = "mxc://example.invalid/avatar";
const USER_ID = "@other:example.invalid";

function avatar(thumbnail: AvatarThumbnailState): AvatarImage {
  return { mxc_uri: AVATAR_URI, thumbnail };
}

function liveSignals(thumbnail: AvatarThumbnailState): LiveSignalsState {
  return {
    rooms: {
      "!room:example.invalid": {
        receipts_by_event: {
          "$message": {
            readers: [
              {
                user_id: USER_ID,
                display_name: "Other",
                original_display_label: "Other",
                avatar: avatar(thumbnail),
                timestamp_ms: 1
              }
            ],
            total_count: 1,
            overflow_count: 0
          }
        },
        fully_read_event_id: null,
        typing_user_ids: [],
        typing_users: []
      }
    },
    presence: {}
  };
}

afterEach(() => {
  cleanup();
  setActiveLocaleProfile("en", "none");
});

describe("authoritative avatar thumbnail convergence", () => {
  test("renders message and receipt images only after Rust profile/live state becomes ready", () => {
    const item = {
      ...message("$message", "Message"),
      sender: USER_ID,
      sender_label: "Other",
      sender_avatar: avatar({ kind: "notRequested" })
    };
    const store = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [item]
      }
    });
    const notRequested = { kind: "notRequested" } as const;
    const ready = {
      kind: "ready",
      source_ref: "https://example.invalid/avatar.png",
      width: 18,
      height: 18,
      mime_type: "image/png"
    } as const;
    let eventHandler: ((payload: CoreEventPayload) => void) | undefined;
    const transport = baseTransport({
      listenCoreEvents: (handler) => {
        eventHandler = handler;
        return () => undefined;
      }
    });
    const { rerender } = render(
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={transport}
          onReply={vi.fn()}
          profileUsers={{
            [USER_ID]: {
              user_id: USER_ID,
              display_name: "Other",
              display_label: "Other",
              original_display_label: "Other",
              mention_search_terms: [],
              avatar: avatar(notRequested)
            }
          }}
          liveSignals={liveSignals(notRequested)}
        />
      </TimelineStoreContext.Provider>
    );

    expect(document.querySelectorAll(".avatar img")).toHaveLength(0);
    eventHandler?.({
      kind: "Account",
      event: {
        AvatarThumbnailDownloaded: {
          request_id: { connection_id: 1, sequence: 1 },
          mxc_uri: AVATAR_URI,
          thumbnail: ready
        }
      }
    } as CoreEventPayload);
    expect(document.querySelectorAll(".avatar img")).toHaveLength(0);

    rerender(
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={transport}
          onReply={vi.fn()}
          profileUsers={{
            [USER_ID]: {
              user_id: USER_ID,
              display_name: "Other",
              display_label: "Other",
              original_display_label: "Other",
              mention_search_terms: [],
              avatar: avatar(ready)
            }
          }}
          liveSignals={liveSignals(ready)}
        />
      </TimelineStoreContext.Provider>
    );

    expect(document.querySelectorAll(".avatar img, .receipt-reader-avatar img")).toHaveLength(2);
  });
});
