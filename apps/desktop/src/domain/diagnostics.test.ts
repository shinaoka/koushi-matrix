import { describe, expect, test } from "vitest";

import { createBrowserFakeApi } from "../backend/browserFakeApi";
import { diagnosticReport } from "./diagnostics";

describe("diagnosticReport", () => {
  test("summarizes sync, timeline, and crawler progress without private identifiers or message bodies", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.getSnapshot();
    const report = diagnosticReport({
      snapshot: {
        ...snapshot,
        timeline: [
          {
            event_id: "$private-event:example.invalid",
            room_id: "!private-room:example.invalid",
            sender: "@alice:example.invalid",
            timestamp_ms: 1_800_000_000_000,
            body: "secret message body",
            attachment_filename: null,
            reply_count: 0
          }
        ],
        state: {
          ...snapshot.state,
          domain: {
            ...snapshot.state.domain,
            search_crawler: {
              rooms: {
                "!private-room:example.invalid": {
                  kind: "running",
                  processed: 42,
                  indexed: 24
                }
              }
            }
          },
          ui: {
            ...snapshot.state.ui,
            errors: [
              {
                code: "timeline_subscription_failed",
                message: "SDK detail for !private-room:example.invalid",
                recoverable: true
              }
            ]
          }
        }
      },
      panelMode: "closed",
      sendStatus: "idle",
      timelineDiagnostics: {
        visibleItems: 3,
        downloadedItems: 7,
        backfill: "paginating"
      },
      domDiagnostics: {
        screen: "timeline",
        rootChildren: 1,
        bodyTextLength: 99
      }
    });

    expect(report).toContain("Koushi diagnostics");
    expect(report).toContain("Timeline visible items: 3");
    expect(report).toContain("Downloading messages from 1 room(s): processed=42 indexed=24");
    expect(report).toContain("Latest error code: timeline_subscription_failed");
    expect(report).not.toContain("secret message body");
    expect(report).not.toContain("!private-room");
    expect(report).not.toContain("@alice");
    expect(report).not.toContain("$private-event");
    expect(report).not.toContain("SDK detail");
  });
});
