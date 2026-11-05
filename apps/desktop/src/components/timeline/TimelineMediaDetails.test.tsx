// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { TimelineMediaAttachment } from "./TimelineMedia";
import type { TimelineItem } from "../../domain/coreEvents";
import { t } from "../../i18n/messages";

afterEach(cleanup);

const media: NonNullable<TimelineItem["media"]> = {
  kind: "Image",
  filename: "synthetic_image.png",
  source: { mxc_uri: "mxc://example.invalid/synthetic", encrypted: false, encryption_version: null },
  mimetype: "image/png",
  size: 2048,
  width: 640,
  height: 480,
  thumbnail: null
};

const viewerEntry = {
  eventId: "$synthetic-media:example.invalid",
  filename: "synthetic_photo.png",
  kind: "Image" as const,
  size: 2048,
  width: 640,
  height: 480,
  mimetype: "image/png",
  encrypted: false,
  saveName: null
};

function attachment(filename: string) {
  return (
    <TimelineMediaAttachment
      media={{ ...media, filename }}
      progress={null}
      canDownload={false}
      onDownload={vi.fn()}
      onOpenMediaViewer={vi.fn()}
      saveName={null}
      viewerEntry={viewerEntry}
    />
  );
}

function detailsTrigger(filename: string): HTMLElement {
  return screen.getByRole("button", { name: t("timeline.mediaDetails", { filename }) });
}

function popovers(): NodeListOf<Element> {
  return document.querySelectorAll(".message-media-details-popover");
}

// #1368: media details render in the body-level floating layer without a
// backdrop, so an outside press has to dismiss them; opening one row's details
// closes any other row's.
describe("media details popup dismissal", () => {
  it("dismisses the details popup on a press outside it", () => {
    render(attachment("synthetic_image.png"));

    fireEvent.click(detailsTrigger("synthetic_image.png"));
    expect(popovers()).toHaveLength(1);

    fireEvent.pointerDown(document.body);

    expect(popovers()).toHaveLength(0);
    expect(detailsTrigger("synthetic_image.png").getAttribute("aria-expanded")).toBe("false");
  });

  it("keeps the details popup while the press lands inside it", () => {
    render(attachment("synthetic_image.png"));

    fireEvent.click(detailsTrigger("synthetic_image.png"));
    fireEvent.pointerDown(popovers()[0]!);

    expect(popovers()).toHaveLength(1);
  });

  it("opens one media details popup at a time", () => {
    render(
      <>
        {attachment("synthetic_one.png")}
        {attachment("synthetic_two.png")}
      </>
    );

    fireEvent.click(detailsTrigger("synthetic_one.png"));
    fireEvent.click(detailsTrigger("synthetic_two.png"));

    expect(popovers()).toHaveLength(1);
    expect(detailsTrigger("synthetic_one.png").getAttribute("aria-expanded")).toBe("false");
    expect(detailsTrigger("synthetic_two.png").getAttribute("aria-expanded")).toBe("true");
  });
});
