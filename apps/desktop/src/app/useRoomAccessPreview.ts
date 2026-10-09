import { useEffect, useState } from "react";

import type { DesktopApi } from "../backend/desktopApi";
import type {
  CreateRoomAccessPreview,
  CreateRoomAccessPreviewInput,
  RoomAccessDraftScope,
  RoomAccessPreview,
  RoomAccessPreviewContext
} from "../domain/types";

/** The identity of one preview request: scope, context and resolved inputs. */
function previewIdentity(
  scope: RoomAccessDraftScope | null,
  context: string,
  stateGeneration: number | undefined,
  extra = ""
): string {
  return `${scope ? JSON.stringify(scope) : ""}|${context}|${stateGeneration ?? ""}|${extra}`;
}

/**
 * Issue #1177: render the Rust access/history preview for one panel, never an
 * older response and never a result for a different identity.
 *
 * The preview is fenced by the full identity: the scope/context arguments and
 * the published `stateGeneration`, which advances for every draft mutation, a
 * confirmed-property advance, an encryption toggle or a session reset. A result
 * is returned only while its identity still matches, so a change to the
 * selection never renders the previous selection's details or verdict.
 */
export function useRoomAccessPreview(
  api: Pick<DesktopApi, "previewRoomAccess">,
  scope: RoomAccessDraftScope | null,
  context: RoomAccessPreviewContext,
  stateGeneration: number | undefined,
  enabled: boolean
): RoomAccessPreview | null {
  const identity = previewIdentity(scope, context, stateGeneration);
  const [result, setResult] = useState<{ identity: string; preview: RoomAccessPreview } | null>(
    null
  );
  useEffect(() => {
    if (!enabled || !scope) {
      return;
    }
    let current = true;
    const requestIdentity = identity;
    void api
      .previewRoomAccess(scope, context)
      .then((preview) => {
        if (current) setResult({ identity: requestIdentity, preview });
      })
      .catch(() => {
        if (current) setResult(null);
      });
    return () => {
      current = false;
    };
  }, [api, scope, context, stateGeneration, enabled, identity]);
  return enabled && scope && result?.identity === identity ? result.preview : null;
}

/**
 * Issue #1177: render the Rust effective-proposed-tuple preview for the create
 * dialog, fenced the same way. The input is a plain value object, so it is
 * keyed by value while the effect still re-requests on every published
 * generation.
 */
export function useCreateRoomAccessPreview(
  api: Pick<DesktopApi, "previewCreateRoomAccess">,
  scope: RoomAccessDraftScope | null,
  input: CreateRoomAccessPreviewInput,
  stateGeneration: number | undefined,
  enabled: boolean
): CreateRoomAccessPreview | null {
  const inputKey = JSON.stringify(input);
  const identity = previewIdentity(scope, "create", stateGeneration, inputKey);
  const [result, setResult] = useState<{
    identity: string;
    preview: CreateRoomAccessPreview;
  } | null>(null);
  useEffect(() => {
    if (!enabled || !scope) {
      return;
    }
    let current = true;
    const requestIdentity = identity;
    void api
      .previewCreateRoomAccess(scope, JSON.parse(inputKey) as CreateRoomAccessPreviewInput)
      .then((preview) => {
        if (current) setResult({ identity: requestIdentity, preview });
      })
      .catch(() => {
        if (current) setResult(null);
      });
    return () => {
      current = false;
    };
  }, [api, scope, inputKey, stateGeneration, enabled, identity]);
  return enabled && scope && result?.identity === identity ? result.preview : null;
}
