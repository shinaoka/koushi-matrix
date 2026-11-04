// @vitest-environment jsdom

import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import type { ReplyQuote } from "../domain/coreEvents";
import type { ComposerDocument } from "../domain/types";
import { setActiveLocaleProfile, t } from "../i18n/messages";
import { Composer } from "./composer";

const noop = () => undefined;
const asyncNoop = async () => undefined;
const resolveNoop = async () => "noop" as const;
const document: ComposerDocument = { version: 2, inlines: [] };

const quote: ReplyQuote = {
  event_id: "$picture:example.invalid",
  sender: "@alice:example.invalid",
  sender_label: "Alice",
  body_preview: null,
  media: {
    kind: "image",
    animated: false,
    width: 640,
    height: 480,
    has_thumbnail_source: false
  },
  permalink: null,
  can_open: false,
  state: "ready"
};

afterEach(() => {
  cleanup();
  setActiveLocaleProfile("en", "none");
});

describe("composer reply quote", () => {
  it("shows the sender, the localized kind label, and a 32 px thumbnail strip", () => {
    const { container } = render(
      <Composer
        composerMode={{ kind: "reply", in_reply_to_event_id: quote.event_id }}
        isSending={false}
        roomName="Lab"
        document={document}
        draftKey="!lab"
        onAttachFiles={asyncNoop}
        onCancelReply={noop}
        onSend={noop}
        onDocumentChange={noop}
        resolveComposerKeyAction={resolveNoop}
        replyQuote={quote}
        replyQuoteThumbnail={{
          kind: "ready",
          source_ref: "download/1234567890123456/6543210987654321/96x96",
          width: 96,
          height: 96,
          mime_type: "image/jpeg"
        }}
      />
    );

    expect(screen.getByText("Alice")).toBeTruthy();
    expect(screen.getByText(t("timeline.replyQuoteKindImage"))).toBeTruthy();
    const slot = container.querySelector(".composer-reply-quote .reply-quote-thumbnail.is-composer");
    expect(slot).not.toBeNull();
    expect(slot?.querySelector("img")).not.toBeNull();
    expect(
      screen.getByRole("button", { name: t("composer.cancelReply") })
    ).toBeTruthy();
  });

  it("falls back to the existing replying label without a quote projection", () => {
    render(
      <Composer
        composerMode={{ kind: "reply", in_reply_to_event_id: quote.event_id }}
        isSending={false}
        roomName="Lab"
        document={document}
        draftKey="!lab"
        onAttachFiles={asyncNoop}
        onCancelReply={noop}
        onSend={noop}
        onDocumentChange={noop}
        resolveComposerKeyAction={resolveNoop}
      />
    );

    expect(screen.getByText(t("composer.replying"))).toBeTruthy();
  });
});
