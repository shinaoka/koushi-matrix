/**
 * Headless spec: an avatar image is clipped once, with no colour behind it
 * (#2006).
 *
 * The report: timeline sender avatars carried a thin ring that broke into
 * near-white gaps at 0°, 90°, 180° and 270°, measured on a 40 px avatar at 2x as
 * ~200/255 across most of the rim against ~244/255 at those four points. The
 * slate fallback colour was painted under an image whose antialiased edge does
 * not cover its own box, and `.avatar-content` added a second rounded clip that
 * squared the image's coverage under the first.
 *
 * Pixels are not asserted here: the rim's rasterisation belongs to the engine,
 * and a screenshot comparison would report the engine rather than the
 * stylesheet. What is asserted is the pair of facts that decides the rim — one
 * clip, and no colour behind an image — read from the computed styles of a
 * message avatar the app itself rendered, so the `.message`/`.timeline-view`
 * cascade applies as it does in production.
 */

import { expect, test, type Page } from "@playwright/test";

import { seedTimelineItems } from "./support/basicOperations";

const SENDER = "@avatar-sender:example.invalid";
const SENDER_LABEL = "Avatar Sender";
/** 1x1 white GIF: the near-white avatar the report says shows the ring best. */
const WHITE_AVATAR =
  "data:image/gif;base64,R0lGODlhAQABAPAAAP///wAAACH5BAAAAAAALAAAAAABAAEAAAICRAEAOw==";

type AvatarStyles = Readonly<{
  borderRadius: string;
  borderWidth: string;
  boxShadow: string;
  overflow: string;
  backgroundColor: string;
  contentOverflow: string;
}>;

/** Gives `SENDER` the avatar (or none, for initials) in the profile the rows read. */
async function seedSenderProfile(page: Page, withImage: boolean): Promise<void> {
  await page.evaluate(
    ({ sender, label, withImage, source }) => {
      const avatar = withImage
        ? {
            mxc_uri: "mxc://example.invalid/avatar-sender",
            thumbnail: { kind: "ready" as const, source_ref: source, width: 1, height: 1, mime_type: "image/gif" }
          }
        : null;
      const snapshot = window.__harness.currentSnapshot();
      window.__harness.setSnapshot({
        ...snapshot,
        state: {
          ...snapshot.state,
          domain: {
            ...snapshot.state.domain,
            profile: {
              ...snapshot.state.domain.profile,
              users: {
                ...snapshot.state.domain.profile.users,
                [sender]: {
                  user_id: sender,
                  display_name: label,
                  display_label: label,
                  original_display_label: label,
                  mention_search_terms: [label],
                  avatar
                }
              }
            }
          }
        }
      });
      window.__harness.pushStateUpdate();
    },
    { sender: SENDER, label: SENDER_LABEL, withImage, source: WHITE_AVATAR }
  );
}

async function seedSenderMessage(page: Page, eventId: string, body: string): Promise<void> {
  await seedTimelineItems(page, [
    {
      id: { Event: { event_id: eventId } },
      sender: SENDER,
      sender_label: SENDER_LABEL,
      body,
      timestamp_ms: 1_800_000_400_000,
      in_reply_to_event_id: null,
      thread_root: null,
      thread_summary: null,
      media: null,
      reactions: [],
      can_react: true,
      is_redacted: false,
      is_hidden: false,
      can_redact: false,
      is_edited: false,
      can_edit: false
    }
  ]);
}

async function avatarStyles(row: ReturnType<Page["locator"]>): Promise<AvatarStyles> {
  return row.locator(".avatar").evaluate((avatar: Element) => {
    const content = avatar.querySelector(".avatar-content");
    if (!content) {
      throw new Error("the message avatar has no .avatar-content");
    }
    const outer = getComputedStyle(avatar);
    return {
      borderRadius: outer.borderRadius,
      borderWidth: outer.borderTopWidth,
      boxShadow: outer.boxShadow,
      overflow: outer.overflow,
      backgroundColor: outer.backgroundColor,
      contentOverflow: getComputedStyle(content).overflow
    };
  });
}

test("an avatar image is clipped once and has no colour behind it (#2006)", async ({ page }) => {
  await page.goto("/appHarness.html");
  await seedSenderProfile(page, true);
  await seedSenderMessage(page, "$avatar-image-event:example.invalid", "Avatar-backed message");

  const row = page.locator('[data-event-id="$avatar-image-event:example.invalid"]');
  await expect(row.locator(".avatar img")).toHaveAttribute("src", /data:image\/gif;base64/);

  const avatar = await avatarStyles(row);
  // The circle is the clip: a border or an inset shadow would each be a second
  // edge to antialias, which is the shape the report describes.
  expect(avatar.borderRadius).toBe("50%");
  expect(avatar.overflow).toBe("hidden");
  expect(avatar.borderWidth).toBe("0px");
  expect(avatar.boxShadow).toBe("none");
  // The image is clipped by the avatar alone, and the fallback colour is not
  // painted under it.
  expect(avatar.contentOverflow).toBe("visible");
  expect(avatar.backgroundColor).toBe("rgba(0, 0, 0, 0)");
});

test("an initials avatar keeps the fallback colour (#2006)", async ({ page }) => {
  await page.goto("/appHarness.html");
  await seedSenderProfile(page, false);
  await seedSenderMessage(page, "$avatar-initials-event:example.invalid", "Initials-backed message");

  const row = page.locator('[data-event-id="$avatar-initials-event:example.invalid"]');
  await expect(row.locator(".avatar .avatar-fallback")).toBeVisible();

  const avatar = await avatarStyles(row);
  expect(avatar.borderRadius).toBe("50%");
  expect(avatar.overflow).toBe("hidden");
  expect(avatar.borderWidth).toBe("0px");
  expect(avatar.boxShadow).toBe("none");
  // `:has(img)` must not reach an avatar whose content is initials.
  expect(avatar.backgroundColor).not.toBe("rgba(0, 0, 0, 0)");
});
