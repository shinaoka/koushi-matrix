import { describe, expect, test } from "vitest";

import type { LiveReadReceipt } from "../domain/types";
import { receiptDisplayName } from "./TimelineView";

function receipt(overrides: Partial<LiveReadReceipt> = {}): LiveReadReceipt {
  return {
    user_id: "@private:example.invalid",
    display_name: "Visible Reader",
    original_display_label: "Original Reader",
    avatar: null,
    timestamp_ms: null,
    ...overrides
  };
}

describe("TimelineView receipt display", () => {
  test("uses Rust-projected original label instead of raw user id fallback", () => {
    expect(
      receiptDisplayName(
        receipt({
          display_name: "  ",
          original_display_label: "Upstream Reader"
        })
      )
    ).toBe("Upstream Reader");
  });

  test("does not expose raw user id when receipt labels are blank", () => {
    expect(
      receiptDisplayName(
        receipt({
          display_name: " ",
          original_display_label: " "
        })
      )
    ).toBe("");
  });
});
