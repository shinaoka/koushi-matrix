import { describe, expect, test } from "vitest";

import { createBrowserFakeApi } from "../backend/browserFakeApi";
import {
  qaSendCompletionStatusFromCoreEvent,
  qaSendSmokeCanStart,
  qaSendSmokeCompletionStatus,
  qaSendSmokeMessageFromEnv,
  qaSendSmokeTargetDiagnosticTokens,
  qaSendSmokeTargetRoom,
  qaSendSmokeTargetUserIdFromEnv
} from "./qaSendSmoke";

describe("qaSendSmoke", () => {
  test("normalizes the synthetic send smoke message from env", () => {
    expect(qaSendSmokeMessageFromEnv("  Synthetic QA message  ")).toBe("Synthetic QA message");
    expect(qaSendSmokeMessageFromEnv("   ")).toBeNull();
    expect(qaSendSmokeMessageFromEnv(undefined)).toBeNull();
  });

  test("normalizes an optional synthetic send target user id", () => {
    expect(qaSendSmokeTargetUserIdFromEnv("  @hiroshi.shinaoka:matrix.org  ")).toBe(
      "@hiroshi.shinaoka:matrix.org"
    );
    expect(qaSendSmokeTargetUserIdFromEnv("hiroshi.shinaoka:matrix.org")).toBe(
      "@hiroshi.shinaoka:matrix.org"
    );
    expect(qaSendSmokeTargetUserIdFromEnv("   ")).toBeNull();
    expect(qaSendSmokeTargetUserIdFromEnv(undefined)).toBeNull();
  });

  test("finds the DM room for a synthetic send target without exposing room names", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.startDirectMessage("@hiroshi.shinaoka:matrix.org");

    const room = qaSendSmokeTargetRoom(snapshot, "@hiroshi.shinaoka:matrix.org");

    expect(room?.is_dm).toBe(true);
    expect(room?.dm_user_ids).toContain("@hiroshi.shinaoka:matrix.org");
  });

  test("summarizes target DM encryption without exposing identifiers", async () => {
    const api = createBrowserFakeApi();
    const started = await api.startDirectMessage("@hiroshi.shinaoka:matrix.org");
    const room = qaSendSmokeTargetRoom(started, "@hiroshi.shinaoka:matrix.org");
    expect(room).not.toBeNull();
    const encryptedSnapshot = {
      ...started,
      state: {
        ...started.state,
        domain: {
          ...started.state.domain,
          rooms: started.state.domain.rooms.map((candidate) =>
            candidate.room_id === room?.room_id
              ? { ...candidate, is_encrypted: true, joined_members: 2 }
              : candidate
          )
        }
      }
    };

    const tokens = qaSendSmokeTargetDiagnosticTokens(
      encryptedSnapshot,
      "@hiroshi.shinaoka:matrix.org"
    );

    expect(tokens).toEqual(["target_dm=encrypted", "target_selected=true", "target_members=2"]);
    expect(tokens.join(" ")).not.toContain("@");
    expect(tokens.join(" ")).not.toContain("!");
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
          ui: {
            ...snapshot.state.ui,
            errors: [
              {
                code: "synthetic_error",
                message: "Synthetic error",
                recoverable: true
              }
            ]
          }
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
        ui: {
          ...snapshot.state.ui,
          timeline: {
            ...snapshot.state.ui.timeline,
            composer: {
              ...snapshot.state.ui.timeline.composer,
              pending_transaction_id: null
            }
          }
        }
      }
    };
    const sending = {
      ...snapshot,
      state: {
        ...snapshot.state,
        ui: {
          ...snapshot.state.ui,
          timeline: {
            ...snapshot.state.ui.timeline,
            composer: {
              ...snapshot.state.ui.timeline.composer,
              pending_transaction_id: "txn1"
            }
          }
        }
      }
    };
    const failed = {
      ...snapshot,
      state: {
        ...snapshot.state,
        ui: {
          ...snapshot.state.ui,
          errors: [
            {
              code: "send_text_failed",
              message: "Matrix send failed",
              recoverable: true
            }
          ]
        }
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
