import { describe, expect, test } from "vitest";

import { createBrowserFakeApi } from "../backend/browserFakeApi";
import {
  qaSendCompletionStatusFromCoreEvent,
  qaSendSmokeCanStart,
  qaSendSmokeCompletionStatus,
  qaSendSmokeMessageFromEnv
} from "./qaSendSmoke";

describe("qaSendSmoke", () => {
  test("normalizes the synthetic send smoke message from env", () => {
    expect(qaSendSmokeMessageFromEnv("  Synthetic QA message  ")).toBe("Synthetic QA message");
    expect(qaSendSmokeMessageFromEnv("   ")).toBeNull();
    expect(qaSendSmokeMessageFromEnv(undefined)).toBeNull();
  });

  test("starts only after a ready synced active timeline without errors", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.getSnapshot();

    expect(qaSendSmokeCanStart(snapshot)).toBe(true);

    expect(
      qaSendSmokeCanStart({
        ...snapshot,
        state: {
          ...snapshot.state,
          errors: [
            {
              code: "synthetic_error",
              message: "Synthetic error",
              recoverable: true
            }
          ]
        }
      })
    ).toBe(false);
  });

  test("marks send completion from pending state and error count", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.getSnapshot();
    const idle = {
      ...snapshot,
      timeline: [],
      state: {
        ...snapshot.state,
        timeline: {
          ...snapshot.state.timeline,
          composer: {
            ...snapshot.state.timeline.composer,
            pending_transaction_id: null
          }
        }
      }
    };
    const sending = {
      ...snapshot,
      state: {
        ...snapshot.state,
        timeline: {
          ...snapshot.state.timeline,
          composer: {
            ...snapshot.state.timeline.composer,
            pending_transaction_id: "txn1"
          }
        }
      }
    };
    const failed = {
      ...snapshot,
      state: {
        ...snapshot.state,
        errors: [
          {
            code: "send_text_failed",
            message: "Matrix send failed",
            recoverable: true
          }
        ]
      }
    };

    expect(qaSendSmokeCompletionStatus(idle, 0)).toBe("idle");
    expect(qaSendSmokeCompletionStatus(sending, 0)).toBe("pending");
    expect(qaSendSmokeCompletionStatus(snapshot, 0)).toBe("sent");
    expect(qaSendSmokeCompletionStatus(failed, 0)).toBe("failed");
  });

  test("maps Tauri CoreEvent send completion to QA send statuses", () => {
    expect(
      qaSendCompletionStatusFromCoreEvent({
        kind: "Timeline",
        event: {
          SendCompleted: {
            request_id: { connection_id: 1, sequence: 2 },
            key: {
              account_key: "@qa:localhost",
              kind: { Room: { room_id: "!room:localhost" } }
            },
            transaction_id: "txn1",
            event_id: "$event:localhost"
          }
        }
      })
    ).toBe("sent");

    expect(
      qaSendCompletionStatusFromCoreEvent({
        kind: "OperationFailed",
        request_id: { connection_id: 1, sequence: 2 },
        failure: { TimelineOperationFailed: { kind: "Sdk" } }
      })
    ).toBe("failed");

    expect(qaSendCompletionStatusFromCoreEvent({ kind: "Sync", event: "Running" })).toBeNull();
  });
});
