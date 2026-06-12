import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, test } from "vitest";

import { ContextMenuSurface, contextMenuPosition } from "./ContextMenuSurface";

describe("ContextMenuSurface", () => {
  test("renders context menu items with destructive affordance and fixed position", () => {
    const markup = renderToStaticMarkup(
      <ContextMenuSurface
        items={[
          { id: "openThread", label: "Reply in thread" },
          { id: "redactMessage", label: "Redact", destructive: true }
        ]}
        x={120}
        y={80}
        onAction={() => undefined}
        onClose={() => undefined}
      />
    );

    expect(markup).toContain('role="menu"');
    expect(markup).toContain("Reply in thread");
    expect(markup).toContain("Redact");
    expect(markup).toContain("context-menu-item destructive");
    expect(markup).toContain("left:120px");
    expect(markup).toContain("top:80px");
  });

  test("clamps context menus inside the viewport", () => {
    expect(
      contextMenuPosition({
        itemCount: 3,
        viewport: { height: 768, width: 1024 },
        x: 980,
        y: 740
      })
    ).toEqual({ left: 832, top: 656 });
  });
});
