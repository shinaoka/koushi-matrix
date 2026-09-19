// @vitest-environment jsdom
import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { CoreEventPayload } from "../domain/coreEvents";
import type { LinkPreview } from "../domain/linkPreview";
import { KEY, baseTransport, message } from "./timelineViewTestSupport";
import { TimelineView, clearTimelineViewportSessionMemoryForTests } from "./TimelineView";

afterEach(() => {
  cleanup();
  clearTimelineViewportSessionMemoryForTests();
});

it.each<LinkPreview>([
  { url: "https://example.invalid/article", state: "pending" },
  { url: "https://example.invalid/article", state: "loading" },
  { url: "https://example.invalid/article", state: "failed" },
  { url: "https://example.invalid/article", state: "ready" },
  { url: "https://example.invalid/article", state: "ready", title: "  ", description: "\n" }
])("skips empty preview cards for $state while retaining the message", async (preview) => {
  let emit: (payload: CoreEventPayload) => void = () => undefined;
  const transport = baseTransport({
    listenCoreEvents(listener) { emit = listener; return () => undefined; }
  });
  const { container } = render(
    <TimelineView timelineKey={KEY} roomId="!room:example.invalid" transport={transport} onReply={vi.fn()} />
  );
  const publish = (linkPreview: LinkPreview, generation: number) => act(() => emit({
    kind: "Timeline",
    event: { InitialItems: { request_id: null, key: KEY, generation,
      items: [{ ...message("$preview:example.invalid", "Synthetic link message"), link_previews: [linkPreview] }]
    } }
  }));
  publish(preview, 1);
  await screen.findByText("Synthetic link message");
  expect(container.querySelector(".link-preview-cards")).toBeNull();
  publish({ ...preview, state: "ready", title: "Synthetic article" }, 2);
  await screen.findByRole("link", { name: /Synthetic article/ });
  expect(container.querySelectorAll(".link-preview-card")).toHaveLength(1);
});
