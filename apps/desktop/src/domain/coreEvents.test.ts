// @vitest-environment jsdom

import { describe, expect, it } from "vitest";

import {
  isSecureBackupSendRefusal,
  isUnsupportedSlashCommandRejection,
  noticeMatchesMainComposer,
  noticeMatchesThreadComposer,
  type CoreEventPayload,
  type TimelineKey
} from "./coreEvents";

describe("isUnsupportedSlashCommandRejection (#450)", () => {
  const submissionRejected: CoreEventPayload = {
    kind: "Timeline",
    event: {
      SubmissionRejected: {
        request_id: { connection_id: 1, sequence: 2 },
        key: {
          account_key: "@a:example.invalid",
          kind: { Room: { room_id: "!r:example.invalid" } }
        },
        submission_id: "submission-1",
        kind: "UnsupportedSlashCommand"
      }
    }
  };
  const operationFailed: CoreEventPayload = {
    kind: "OperationFailed",
    request_id: null,
    failure: {
      TimelineOperationFailed: { kind: "UnsupportedSlashCommand" }
    }
  };

  it("recognizes the production SubmissionRejected surface", () => {
    expect(isUnsupportedSlashCommandRejection(submissionRejected)).toBe(true);
  });

  it("recognizes the OperationFailed surface", () => {
    expect(isUnsupportedSlashCommandRejection(operationFailed)).toBe(true);
  });

  it("ignores other rejection kinds and unrelated payloads", () => {
    expect(
      isUnsupportedSlashCommandRejection({
        kind: "Timeline",
        event: {
          SubmissionRejected: {
            request_id: { connection_id: 1, sequence: 2 },
            key: {
              account_key: "@a:example.invalid",
              kind: { Room: { room_id: "!r:example.invalid" } }
            },
            submission_id: "submission-1",
            kind: "Network"
          }
        }
      } as CoreEventPayload)
    ).toBe(false);
    expect(
      isUnsupportedSlashCommandRejection({
        kind: "OperationFailed",
        request_id: null,
        failure: { TimelineOperationFailed: { kind: "Forbidden" } }
      })
    ).toBe(false);
    expect(
      isUnsupportedSlashCommandRejection({
        kind: "OperationFailed",
        request_id: null,
        failure: { TimelineOperationFailed: { kind: "UnsupportedSlashCommand" } }
      })
    ).toBe(true);
    expect(
      isUnsupportedSlashCommandRejection({
        kind: "Sync",
        event: { Connected: { kind: "connecting" } }
      } as unknown as CoreEventPayload)
    ).toBe(false);
  });
});

describe("composer notice routing (#450)", () => {
  const userId = "@alice:example.invalid";
  const roomKey: TimelineKey = {
    account_key: userId,
    kind: { Room: { room_id: "!room:example.invalid" } }
  };
  const threadKey: TimelineKey = {
    account_key: userId,
    kind: {
      Thread: {
        room_id: "!room:example.invalid",
        root_event_id: "$root:example.invalid"
      }
    }
  };
  const otherAccountKey: TimelineKey = {
    ...roomKey,
    account_key: "@bob:example.invalid"
  };

  it("matches the main composer only for the same room and account", () => {
    expect(noticeMatchesMainComposer(roomKey, "!room:example.invalid", userId)).toBe(true);
    expect(noticeMatchesMainComposer(roomKey, "!other:example.invalid", userId)).toBe(false);
    // Cross-account: the same room under another account never matches.
    expect(noticeMatchesMainComposer(otherAccountKey, "!room:example.invalid", userId)).toBe(false);
    // A thread key never matches the main composer.
    expect(noticeMatchesMainComposer(threadKey, "!room:example.invalid", userId)).toBe(false);
  });

  it("matches the thread composer only for the same thread and account", () => {
    expect(
      noticeMatchesThreadComposer(threadKey, "!room:example.invalid", "$root:example.invalid", userId)
    ).toBe(true);
    // Wrong room and wrong root never match.
    expect(
      noticeMatchesThreadComposer(threadKey, "!other-room:example.invalid", "$root:example.invalid", userId)
    ).toBe(false);
    expect(
      noticeMatchesThreadComposer(threadKey, "!room:example.invalid", "$other-root:example.invalid", userId)
    ).toBe(false);
    // Cross-account: the same thread under another account never matches.
    expect(
      noticeMatchesThreadComposer(
        { ...threadKey, account_key: "@bob:example.invalid" },
        "!room:example.invalid",
        "$root:example.invalid",
        userId
      )
    ).toBe(false);
    // Cross-kind: a room key never matches the thread composer.
    expect(
      noticeMatchesThreadComposer(roomKey, "!room:example.invalid", "$root:example.invalid", userId)
    ).toBe(false);
    expect(noticeMatchesMainComposer(threadKey, "!room:example.invalid", userId)).toBe(false);
  });
});

describe("isSecureBackupSendRefusal (#2066)", () => {
  const refusal = (kind: string): CoreEventPayload => ({
    kind: "OperationFailed",
    request_id: null,
    failure: { TimelineOperationFailed: { kind } as never }
  });

  it("names the Secure Backup gate Rust answers every send-family refusal with", () => {
    expect(isSecureBackupSendRefusal(refusal("SecureBackupRequired"))).toBe(true);
  });

  it("leaves the other timeline failures to their own handling", () => {
    expect(isSecureBackupSendRefusal(refusal("UnsupportedSlashCommand"))).toBe(false);
    expect(isSecureBackupSendRefusal(refusal("Forbidden"))).toBe(false);
    expect(
      isSecureBackupSendRefusal({ kind: "OperationFailed", request_id: null, failure: "SessionRequired" })
    ).toBe(false);
    expect(isSecureBackupSendRefusal({ kind: "ResyncMarker" })).toBe(false);
  });
});
