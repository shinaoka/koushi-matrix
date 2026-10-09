import { useEffect, useState } from "react";

import type { DesktopApi } from "../backend/desktopApi";
import type {
  CreateRoomAccessPreview,
  CreateRoomAccessPreviewInput,
  RoomAccessDraftScope,
  RoomAccessPreview,
  RoomAccessPreviewContext
} from "../domain/types";

/**
 * Issue #1177: render the Rust access/history preview for one panel, never an
 * older response.
 *
 * The preview is fenced by the full identity: the scope/context arguments, and
 * the published `stateGeneration`, which advances for every draft mutation, a
 * confirmed-property advance, an encryption toggle or a session reset. The
 * effect cleanup drops a response from any earlier identity, so an out-of-order
 * result can never replace newer details.
 */
export function useRoomAccessPreview(
  api: Pick<DesktopApi, "previewRoomAccess">,
  scope: RoomAccessDraftScope | null,
  context: RoomAccessPreviewContext,
  stateGeneration: number | undefined,
  enabled: boolean
): RoomAccessPreview | null {
  const [result, setResult] = useState<RoomAccessPreview | null>(null);
  useEffect(() => {
    let current = true;
    if (!enabled || !scope) {
      setResult(null);
      return () => {
        current = false;
      };
    }
    void api
      .previewRoomAccess(scope, context)
      .then((preview) => {
        if (current) setResult(preview);
      })
      .catch(() => {
        if (current) setResult(null);
      });
    return () => {
      current = false;
    };
  }, [api, scope, context, stateGeneration, enabled]);
  return result;
}

/**
 * Issue #1177: render the Rust effective-proposed-tuple preview for the create
 * dialog, never an older response. The input is a plain value object, so it is
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
  const [result, setResult] = useState<CreateRoomAccessPreview | null>(null);
  const inputKey = JSON.stringify(input);
  useEffect(() => {
    let current = true;
    if (!enabled || !scope) {
      setResult(null);
      return () => {
        current = false;
      };
    }
    void api
      .previewCreateRoomAccess(scope, JSON.parse(inputKey) as CreateRoomAccessPreviewInput)
      .then((preview) => {
        if (current) setResult(preview);
      })
      .catch(() => {
        if (current) setResult(null);
      });
    return () => {
      current = false;
    };
  }, [api, scope, inputKey, stateGeneration, enabled]);
  return result;
}
