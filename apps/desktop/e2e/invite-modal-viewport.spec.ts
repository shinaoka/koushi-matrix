/**
 * Headless geometry regression: the Invite people dialog must stay inside the
 * application viewport and keep its actions reachable at every window size
 * (#488).
 *
 * The full invite workflow (selected target + candidate results + history
 * guidance + recovery warning + Room Info link + both scope choices) renders
 * taller than a short packaged window. Before the fix the form had no
 * viewport-bounded max-height and no internal scroll region, so its bottom —
 * including Cancel / Send invite — was clipped off the window with no way to
 * reach it except resizing the whole window.
 *
 * These tests drive the real user flow through the harness (Room info →
 * Invite people), seed the full workflow through the real command responses,
 * and measure rendered geometry, so they fail on the symptom (actions outside
 * the viewport) rather than on a hard-coded height.
 */

import { expect, test, type Page } from "@playwright/test";
import { t } from "../src/i18n/messages";

const ROOM_ID = "!harness-room:example.invalid";
/** Short viewport representative of the reported packaged window. */
const SHORT_VIEWPORT = { width: 820, height: 600 };

interface WorkflowSeed {
  candidates: number;
  selected: boolean;
}

/** The full invite workflow: candidates, selected target, all warnings, both scopes. */
function fullWorkflow({ candidates = 12, selected = true }: WorkflowSeed = {}) {
  const candidate = (index: number) => ({
    user_id: `@candidate-${index}:example.invalid`,
    display_label: `Candidate Person ${index} with a fairly long display name`,
    original_display_label: `Candidate Person ${index} with a fairly long display name`,
    avatar: null,
    source: "profile" as const,
    status: "selectable" as const,
    status_message: null
  });
  return {
    query: {
      room_id: ROOM_ID,
      query: "candidate",
      candidates: Array.from({ length: candidates }, (_, index) => candidate(index)),
      explicit_user_id: null
    },
    selected_targets: selected
      ? [{ user_id: "@member-1:example.invalid", display_label: "Member 1", avatar: null }]
      : [],
    scope_plan: {
      room_id: ROOM_ID,
      destination_kind: "room" as const,
      default_scope: { kind: "roomOnly" as const },
      options: [
        { scope: { kind: "roomOnly" as const }, label: "This room only", detail: null },
        {
          scope: { kind: "parentSpaceAndRoom" as const, space_id: "!space:example.invalid" },
          label: "Room and parent space",
          detail: null
        }
      ]
    },
    selected_scope: { kind: "roomOnly" as const },
    history_policy: {
      current_visibility: "worldReadable" as const,
      encrypted: true,
      can_edit: true,
      readiness: "recoveryRequired" as const
    },
    operation: { kind: "idle" as const }
  };
}

async function gotoReadyShell(page: Page): Promise<void> {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: t("timeline.conversation") })).toBeVisible();
}

/** Seed the full invite workflow through the real command responses. */
async function seedInviteWorkflow(page: Page, seed: WorkflowSeed = {}): Promise<void> {
  await page.evaluate(
    ({ roomId, workflow }) => {
      const withWorkflow = (nextWorkflow: unknown) => {
        const snapshot = window.__harness.currentSnapshot();
        const next = {
          ...snapshot,
          state: {
            ...snapshot.state,
            domain: {
              ...snapshot.state.domain,
              invite_workflow: nextWorkflow
            }
          }
        };
        window.__harness.setSnapshot(next);
        return next;
      };
      window.__harness.setCommandResponse("open_invite_workflow", () =>
        withWorkflow(workflow)
      );
      window.__harness.setCommandResponse("search_invite_targets", () =>
        withWorkflow(workflow)
      );
      window.__harness.setCommandResponse("select_invite_target", () =>
        withWorkflow(workflow)
      );
      window.__harness.setCommandResponse("set_invite_scope", () => withWorkflow(workflow));
      window.__harness.setCommandResponse("close_invite_workflow", () =>
        window.__harness.currentSnapshot()
      );
      window.__harness.setCommandResponse("invite_targets", () =>
        window.__harness.currentSnapshot()
      );
    },
    { roomId: ROOM_ID, workflow: fullWorkflow(seed) }
  );
}

/** Open the invite dialog through the real user path: Room info → Invite people. */
async function openInviteDialog(page: Page): Promise<void> {
  await page.getByRole("button", { name: "Room info" }).click();
  await page.getByRole("button", { name: t("room.invitePeople") }).click();
  const dialog = page.getByRole("dialog", { name: /Invite people to/ });
  await expect(dialog).toBeVisible();
}

/** Rendered geometry of the dialog box and its action buttons. */
async function dialogGeometry(page: Page) {
  return page.evaluate(() => {
    const dialog = document.querySelector<HTMLElement>(".invite-target-dialog");
    if (!dialog) {
      return null;
    }
    const box = dialog.getBoundingClientRect();
    const scrollContainer = dialog.querySelector<HTMLElement>(".invite-dialog-body");
    const actions = Array.from(
      dialog.querySelectorAll<HTMLElement>(".dialog-actions button")
    );
    const actionRects = actions.map((action) => action.getBoundingClientRect());
    return {
      viewportWidth: window.innerWidth,
      viewportHeight: window.innerHeight,
      top: box.top,
      bottom: box.bottom,
      scrollContainer: scrollContainer
        ? {
            height: scrollContainer.clientHeight,
            scrollHeight: scrollContainer.scrollHeight,
            scrollTop: scrollContainer.scrollTop,
            overflow: getComputedStyle(scrollContainer).overflowY
          }
        : null,
      actions: actionRects.map((rect) => ({ top: rect.top, bottom: rect.bottom })),
      pageScrollY: window.scrollY,
      backdrop: (() => {
        const overlay = document.querySelector<HTMLElement>(".dialog-overlay");
        if (!overlay) {
          return null;
        }
        const rect = overlay.getBoundingClientRect();
        return { top: rect.top, bottom: rect.bottom, left: rect.left, right: rect.right };
      })()
    };
  });
}

function expectDialogBounded(geometry: NonNullable<Awaited<ReturnType<typeof dialogGeometry>>>, label: string) {
  const { viewportHeight, top, bottom, actions } = geometry;
  expect(top, `${label}: dialog top ${top} escapes the viewport`).toBeGreaterThanOrEqual(0);
  expect(
    bottom,
    `${label}: dialog bottom ${bottom} exceeds viewport height ${viewportHeight}`
  ).toBeLessThanOrEqual(viewportHeight + 1);
  for (const [index, action] of actions.entries()) {
    expect(
      action.bottom,
      `${label}: action ${index} bottom ${action.bottom} exceeds viewport height ${viewportHeight}`
    ).toBeLessThanOrEqual(viewportHeight + 1);
  }
}

test("the full invite workflow stays inside a short viewport with actions reachable", async ({
  page
}) => {
  await page.setViewportSize(SHORT_VIEWPORT);
  await gotoReadyShell(page);
  await seedInviteWorkflow(page);
  await openInviteDialog(page);

  const geometry = await dialogGeometry(page);
  expect(geometry).not.toBeNull();
  expectDialogBounded(geometry!, "full workflow");

  // The dialog must own an internal scroll region: the content overflows it
  // (proves the tall form has somewhere to go) and the region is scrollable.
  expect(geometry!.scrollContainer).not.toBeNull();
  const scroll = geometry!.scrollContainer!;
  expect(scroll.overflow).toBe("auto");
  expect(scroll.scrollHeight).toBeGreaterThan(scroll.height);

  // The backdrop covers the entire application viewport, so background
  // content cannot receive pointer interaction.
  expect(geometry!.backdrop).not.toBeNull();
  expect(geometry!.backdrop!.top).toBeLessThanOrEqual(0);
  expect(geometry!.backdrop!.bottom).toBeGreaterThanOrEqual(geometry!.viewportHeight - 1);
  expect(geometry!.backdrop!.left).toBeLessThanOrEqual(0);
  expect(geometry!.backdrop!.right).toBeGreaterThanOrEqual(geometry!.viewportWidth - 1);
});

test("scrolling inside the dialog moves only the dialog body, never the page", async ({
  page
}) => {
  await page.setViewportSize(SHORT_VIEWPORT);
  await gotoReadyShell(page);
  await seedInviteWorkflow(page);
  await openInviteDialog(page);

  const before = await dialogGeometry(page);
  expect(before!.scrollContainer).not.toBeNull();

  // Wheel-scroll inside the dialog body a few times.
  const body = page.locator(".invite-dialog-body");
  await body.hover();
  for (let index = 0; index < 4; index += 1) {
    await page.mouse.wheel(0, 120);
  }
  await expect
    .poll(async () => (await dialogGeometry(page))!.scrollContainer!.scrollTop)
    .toBeGreaterThan(0);

  const after = await dialogGeometry(page);
  // Only the dialog body scrolled: the page and the backdrop did not move.
  expect(after!.pageScrollY).toBe(0);
  expect(after!.backdrop!.top).toBeLessThanOrEqual(0);
  expect(after!.backdrop!.bottom).toBeGreaterThanOrEqual(after!.viewportHeight - 1);
});

test("the dialog stays bounded and usable at increased text size", async ({ page }) => {
  await page.setViewportSize(SHORT_VIEWPORT);
  await gotoReadyShell(page);
  await seedInviteWorkflow(page);
  await openInviteDialog(page);

  // Simulate a larger text-size/zoom layout without changing the viewport.
  await page.evaluate(() => {
    document.documentElement.style.fontSize = "20px";
  });

  const geometry = await dialogGeometry(page);
  expect(geometry).not.toBeNull();
  expectDialogBounded(geometry!, "increased text size");
});

test("Tab, Shift+Tab, Escape, and focus restoration keep the dialog usable", async ({
  page
}) => {
  await page.setViewportSize(SHORT_VIEWPORT);
  await gotoReadyShell(page);
  await seedInviteWorkflow(page);
  await openInviteDialog(page);

  // Initial focus lands on the invite search field.
  const searchField = page.getByRole("textbox", { name: t("dialog.inviteSearch") });
  await expect(searchField).toBeFocused();

  // Tab cycles through every control inside the modal (no escape to the page
  // or browser chrome), and the last action — Send invite — is reachable.
  // The dialog has many controls, so walk with a bound and fail if focus ever
  // leaves the overlay.
  const sendInvite = page.getByRole("button", { name: t("dialog.sendInvite") });
  for (let index = 0; index < 40; index += 1) {
    await page.keyboard.press("Tab");
    const active = await page.evaluate(() => {
      const element = document.activeElement as HTMLElement | null;
      return { insideModal: Boolean(element?.closest(".dialog-overlay")), label: element?.textContent ?? "" };
    });
    expect(active.insideModal, "Tab escaped the modal").toBe(true);
    const sendFocused = await sendInvite.evaluate((el) => el === document.activeElement);
    if (sendFocused) {
      break;
    }
  }
  await expect(sendInvite).toBeFocused();

  // Tab past the last control wraps back to the first control (the Remove
  // invite target button, which precedes the search field in DOM order).
  const removeTarget = page.getByRole("button", { name: t("dialog.removeInviteTarget") });
  await page.keyboard.press("Tab");
  await expect(removeTarget).toBeFocused();

  // Shift+Tab past the first control wraps to the last (Send invite).
  await page.keyboard.press("Shift+Tab");
  await expect(sendInvite).toBeFocused();

  // Escape closes the modal.
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog", { name: /Invite people to/ })).not.toBeVisible();

  // Focus returns to the Invite people trigger.
  await expect(page.getByRole("button", { name: t("room.invitePeople") })).toBeFocused();
});

test("ordinary shorter dialogs are not regressed by the modal layout", async ({ page }) => {
  await page.setViewportSize(SHORT_VIEWPORT);
  await gotoReadyShell(page);

  // Open the New DM dialog (a short .dialog-box/.dialog-overlay dialog) from
  // the Invites view, the same path the other headless specs use.
  await page.getByRole("navigation", { name: "Workspaces" }).getByRole("button", { name: /^Home/ }).click();
  await page.getByRole("button", { name: "Invites", exact: true }).click();
  await page.getByRole("main", { name: "Invites" }).getByRole("button", { name: "New DM" }).click();
  const dialog = page.getByRole("dialog", { name: t("dialog.newDmTitle") });
  await expect(dialog).toBeVisible();

  const geometry = await page.evaluate(() => {
    const box = document.querySelector<HTMLElement>(".dialog-box");
    if (!box) {
      return null;
    }
    const rect = box.getBoundingClientRect();
    return { top: rect.top, bottom: rect.bottom, viewportHeight: window.innerHeight };
  });
  expect(geometry).not.toBeNull();
  expect(geometry!.top).toBeGreaterThanOrEqual(0);
  expect(geometry!.bottom).toBeLessThanOrEqual(geometry!.viewportHeight + 1);
  await expect(page.getByRole("button", { name: t("dialog.startDm") })).toBeVisible();
});

// #1027: shortening the visible footer keeps recipient context accessible.
for (const locale of ["en", "ja"] as const) {
  test(`invite footer fits with concise actions in ${locale}`, async ({ page }) => {
    await page.setViewportSize(SHORT_VIEWPORT);
    await gotoReadyShell(page);
    await seedInviteWorkflow(page);
    await page.evaluate((locale) => {
      const snapshot = window.__harness.currentSnapshot();
      snapshot.state.domain.locale_profile = { ...snapshot.state.domain.locale_profile, lang: locale, catalog_locale: locale };
      window.__harness.setSnapshot(snapshot);
      window.__harness.pushStateUpdate();
    }, locale);
    await page.getByRole("button", { name: t("room.roomInfo", {}, locale), exact: true }).click();
    await page.getByRole("button", { name: t("room.invitePeople", {}, locale) }).click();
    await expect(page.getByRole("button", { name: t("dialog.sendInvite", {}, locale) })).toHaveText(t("action.invite", {}, locale));
    const geometry = await dialogGeometry(page);
    expect(geometry).not.toBeNull();
    expectDialogBounded(geometry!, locale);
  });
}
