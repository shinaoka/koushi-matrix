import { describe, expect, it } from "vitest";
import { activeMentionQuery } from "./uiShared";

describe("activeMentionQuery", () => {
  it.each(["", "Hello ", "\n", "確認をお願いします", "確認をお願いします。", "（", "hello", "\uFFFC", "😀"])(
    "finds a mention directly after %j",
    (prefix) => {
      expect(activeMentionQuery(`${prefix}@alice`)).toEqual({
        start: prefix.length,
        end: prefix.length + 6,
        query: "alice"
      });
    }
  );

  it("uses only the final trigger and preserves UTF-16 offsets", () => {
    expect(activeMentionQuery("@alice 次に@bob")).toEqual({ start: 9, end: 13, query: "bob" });
    expect(activeMentionQuery("確認@")).toEqual({ start: 2, end: 3, query: "" });
  });

  it.each(["ordinary text", "@alice text", "@alice\uFFFC", "mail@example.invalid"])(
    "does not offer a mention for %j",
    (text) => expect(activeMentionQuery(text)).toBeNull()
  );
});
