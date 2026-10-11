import { CircleHelp, Globe2, Hand, LoaderCircle, UserRoundPlus, UsersRound } from "lucide-react";

import { ICON_SIZE } from "../app/uiShared";
import type { RoomAccessGlyph } from "../domain/accessCondition";

/**
 * #1327: the one icon vocabulary every access surface renders. A padlock is
 * reserved for encryption, so participation never uses it.
 */
export function RoomAccessGlyphIcon({
  glyph,
  size = ICON_SIZE.access
}: {
  glyph: RoomAccessGlyph;
  size?: number;
}) {
  switch (glyph) {
    case "globe":
      return <Globe2 size={size} aria-hidden="true" />;
    case "userRoundPlus":
      return <UserRoundPlus size={size} aria-hidden="true" />;
    case "usersRound":
      return <UsersRound size={size} aria-hidden="true" />;
    case "hand":
      return <Hand size={size} aria-hidden="true" />;
    case "checking":
      return <LoaderCircle size={size} aria-hidden="true" />;
    default:
      return <CircleHelp size={size} aria-hidden="true" />;
  }
}
