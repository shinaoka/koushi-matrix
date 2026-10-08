import { expect, type Page } from "@playwright/test";

/**
 * Shared probes for the left Space rail selection indicator (#1206).
 *
 * The rail is a clipped scrollport, so its rendered state (selection vs hover
 * vs keyboard focus) and the tile fit at compact width are only observable
 * from the browser DOM, not from the React component's class list.
 */
export interface RailButtonState {
  backgroundColor: string;
  boxShadow: string;
  outlineStyle: string;
  outlineWidth: string;
}

/** Seeds one active and one idle Space with Home inactive. */
export async function seedSelectionRail(page: Page): Promise<void> {
  await page.evaluate(() => {
    const next = structuredClone(window.__harness.currentSnapshot());
    next.sidebar.account_home.is_active = false;
    next.sidebar.space_rail = [
      {
        space_id: "!active:example.invalid",
        display_name: "Active Space",
        local_icon: null,
        avatar: null,
        unread_count: 0,
        highlight_count: 0,
        is_active: true,
        leave_candidates: []
      },
      {
        space_id: "!idle:example.invalid",
        display_name: "Idle Space",
        local_icon: null,
        avatar: null,
        unread_count: 0,
        highlight_count: 0,
        is_active: false,
        leave_candidates: []
      }
    ];
    window.__harness.setSnapshot(next);
    window.__harness.pushStateUpdate();
  });
  await expect(page.getByRole("button", { name: "Idle Space" })).toBeVisible();
}

/** Reads the computed indicator style of a rail button by accessible name. */
export async function railButtonState(page: Page, name: string): Promise<RailButtonState> {
  return buttonState(page, ".workspace-button", name);
}

/**
 * Reads the Home button, whose accessible name carries the attention counts
 * rather than a fixed label.
 */
export async function homeButtonState(page: Page): Promise<RailButtonState> {
  return buttonState(page, ".workspace-home-button", null);
}

async function buttonState(
  page: Page,
  selector: string,
  label: string | null
): Promise<RailButtonState> {
  const found = await page.evaluate(
    ({ query, name }) => {
      const buttons = Array.from(document.querySelectorAll<HTMLElement>(query));
      const button = name === null
        ? buttons[0]
        : buttons.find((candidate) => candidate.getAttribute("aria-label") === name);
      if (!button) {
        return null;
      }
      const style = getComputedStyle(button);
      return {
        backgroundColor: style.backgroundColor,
        boxShadow: style.boxShadow,
        outlineStyle: style.outlineStyle,
        outlineWidth: style.outlineWidth
      };
    },
    { query: selector, name: label }
  );
  if (!found) {
    throw new Error(`rail button ${selector}${label ? ` (${label})` : ""} is missing`);
  }
  return found;
}

/** Tabs until the rail button with `name` owns keyboard focus. */
export async function focusRailButton(page: Page, name: string): Promise<void> {
  for (let index = 0; index < 80; index += 1) {
    await page.keyboard.press("Tab");
    const focused = await page.evaluate(() =>
      (document.activeElement as HTMLElement | null)?.getAttribute("aria-label")
    );
    if (focused === name) {
      return;
    }
  }
  throw new Error(`could not keyboard-focus ${name}`);
}
