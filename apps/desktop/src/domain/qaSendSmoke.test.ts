import { describe, expect, test } from "vitest";

import { createBrowserFakeApi } from "../backend/browserFakeApi";
import {
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

    expect(qaSendSmokeCompletionStatus(sending, 0)).toBe("sending");
    expect(qaSendSmokeCompletionStatus(snapshot, 0)).toBe("sent");
    expect(qaSendSmokeCompletionStatus(failed, 0)).toBe("failed");
  });
});
