/**
 * Shared rendered-geometry probes for the right context panel (#452, #1119).
 *
 * #452 fixed the OUTER fit: the `.app-grid` column (or overlay) that hosts the
 * panel must stay inside the window. #1119 is the INNER fit: the panel can sit
 * entirely on-screen while one of its own grid/flex tracks grows to a
 * descendant's intrinsic width, pushing its header, close button, and composer
 * off the panel. The engineering-rules "Right Panel And Composer Containment"
 * contract owns both; these probes measure them separately from the DOM the
 * user actually sees, never from a breakpoint number.
 */

import { expect, type Page } from "@playwright/test";

/** The panel's shell, shared by every right-panel mode. */
export const RIGHT_PANEL_SELECTOR = ".thread-pane";

/** Outer-fit geometry used by the #452 checks. */
export async function panelGeometry(page: Page) {
  return page.evaluate((selector) => {
    const panel = document.querySelector<HTMLElement>(selector);
    if (!panel) {
      return null;
    }
    // Every control in the panel must be reachable — the reporter's escape
    // hatch was its close button, but any clipped control is the same fault.
    // Measured by role rather than class so the assertion does not depend on
    // which panel content happens to be open.
    const controls = Array.from(panel.querySelectorAll<HTMLElement>("button"));
    const panelRect = panel.getBoundingClientRect();
    return {
      viewportWidth: window.innerWidth,
      panelLeft: panelRect.left,
      panelRight: panelRect.right,
      controlCount: controls.length,
      widestControlRight: controls.reduce(
        (max, control) => Math.max(max, control.getBoundingClientRect().right),
        0
      )
    };
  }, RIGHT_PANEL_SELECTOR);
}

export async function dragResizer(page: Page, label: string, deltaX: number): Promise<void> {
  const resizer = page.getByRole("button", { name: label });
  const box = await resizer.boundingBox();
  expect(box).not.toBeNull();
  await page.mouse.move(box!.x + box!.width / 2, box!.y + 4);
  await page.mouse.down();
  await page.mouse.move(box!.x + box!.width / 2 + deltaX, box!.y + 4);
  await page.mouse.up();
}

export interface ContainmentProbe {
  /** CSS selector of the container whose inner fit is checked. */
  root: string;
  /**
   * Controls that must be inside the container and the viewport and must be
   * the topmost element at their own centre. Selectors are relative to `root`.
   */
  controls: Record<string, string>;
  /**
   * Selectors, inside the root, of rows whose rendered children must also stay
   * inside the row itself (a toolbar whose controls must not run past its own
   * edge even while the panel still contains them).
   */
  rows?: string[];
}

export interface HitTarget {
  centerX: number;
  centerY: number;
}

export interface ContainmentReport {
  viewportWidth: number;
  /** `overlay` when the panel is a fixed-position sheet, otherwise `inline`. */
  layout: "inline" | "overlay" | "missing";
  violations: string[];
  hitTargets: Record<string, HitTarget>;
}

/**
 * Inner-fit report for a container (a right-panel mode, or a pane with a
 * composer). Collects every violation instead of stopping at the first so a
 * failure names the escaping descendant:
 *
 * - the container lies inside the viewport and has no inline scroll overflow;
 * - every rendered descendant lies inside the container's inline extent, down
 *   to the first descendant that clips inline overflow;
 * - a clipping descendant hides no inline overflow unless it is an explicit
 *   truncation (`text-overflow: ellipsis`) or an intentional local scroller
 *   (`pre`, code, display math);
 * - every rendered child of each named row stays inside that row;
 * - each named control lies inside the container and the viewport, and
 *   `elementFromPoint` at its centre is the control itself or a descendant.
 */
export async function containmentReport(
  page: Page,
  probe: ContainmentProbe
): Promise<ContainmentReport> {
  return page.evaluate(({ root, controls, rows = [] }) => {
    const tolerance = 1;
    const violations: string[] = [];
    const hitTargets: Record<string, { centerX: number; centerY: number }> = {};
    const container = document.querySelector<HTMLElement>(root);
    const viewportWidth = window.innerWidth;
    if (!container) {
      return { viewportWidth, layout: "missing" as const, violations: [`${root} missing`], hitTargets };
    }
    const round = (value: number) => Math.round(value * 10) / 10;
    const describe = (element: Element) => {
      const classes =
        typeof element.className === "string" && element.className.trim().length > 0
          ? `.${element.className.trim().split(/\s+/).join(".")}`
          : "";
      return `${element.tagName.toLowerCase()}${classes}`;
    };
    const containerRect = container.getBoundingClientRect();
    const containerStyle = getComputedStyle(container);
    const layout = containerStyle.position === "fixed" ? ("overlay" as const) : ("inline" as const);
    const span = (rect: DOMRect) => `[${round(rect.left)}, ${round(rect.right)}]`;

    if (containerRect.left < -tolerance || containerRect.right > viewportWidth + tolerance) {
      violations.push(`${root} ${span(containerRect)} outside the ${viewportWidth}px viewport`);
    }
    if (container.scrollWidth > container.clientWidth + tolerance) {
      violations.push(
        `${root} scrollWidth ${container.scrollWidth} exceeds clientWidth ${container.clientWidth}`
      );
    }

    const intentionalLocalScroll = (element: Element) =>
      element.matches("pre, pre *, code, .katex-display, .math-display, [data-inline-scroll]");
    const escaping: string[] = [];
    const walk = (parent: Element) => {
      for (const child of Array.from(parent.children)) {
        const style = getComputedStyle(child);
        if (style.display === "none" || style.position === "fixed") {
          continue;
        }
        const rect = child.getBoundingClientRect();
        if (rect.width > 0 || rect.height > 0) {
          if (rect.left < containerRect.left - tolerance || rect.right > containerRect.right + tolerance) {
            escaping.push(`${describe(child)} ${span(rect)} outside ${root} ${span(containerRect)}`);
          }
        }
        if (style.overflowX !== "visible") {
          const hidden = child.scrollWidth - child.clientWidth;
          if (
            hidden > tolerance &&
            style.textOverflow !== "ellipsis" &&
            !intentionalLocalScroll(child)
          ) {
            escaping.push(
              `${describe(child)} hides ${hidden}px of inline overflow (scrollWidth ${child.scrollWidth})`
            );
          }
          continue;
        }
        walk(child);
      }
    };
    walk(container);
    violations.push(...escaping.slice(0, 8));
    if (escaping.length > 8) {
      violations.push(`... and ${escaping.length - 8} more escaping descendants`);
    }

    for (const rowSelector of rows) {
      for (const row of Array.from(container.querySelectorAll(rowSelector))) {
        const rowRect = row.getBoundingClientRect();
        for (const child of Array.from(row.children)) {
          const style = getComputedStyle(child);
          if (style.display === "none" || style.position === "fixed" || style.position === "absolute") {
            continue;
          }
          const rect = child.getBoundingClientRect();
          if (
            (rect.width > 0 || rect.height > 0) &&
            (rect.left < rowRect.left - tolerance || rect.right > rowRect.right + tolerance)
          ) {
            violations.push(`${describe(child)} ${span(rect)} outside row ${rowSelector} ${span(rowRect)}`);
          }
        }
      }
    }

    for (const [name, selector] of Object.entries(controls)) {
      const control = container.querySelector<HTMLElement>(selector);
      if (!control) {
        violations.push(`control ${name} (${selector}) missing`);
        continue;
      }
      const rect = control.getBoundingClientRect();
      const centerX = rect.left + rect.width / 2;
      const centerY = rect.top + rect.height / 2;
      hitTargets[name] = { centerX, centerY };
      if (rect.width <= 0 || rect.height <= 0) {
        violations.push(`control ${name} has no box`);
        continue;
      }
      if (rect.left < containerRect.left - tolerance || rect.right > containerRect.right + tolerance) {
        violations.push(`control ${name} ${span(rect)} outside ${root} ${span(containerRect)}`);
      }
      if (rect.left < -tolerance || rect.right > viewportWidth + tolerance) {
        violations.push(`control ${name} ${span(rect)} outside the ${viewportWidth}px viewport`);
      }
      const hit = document.elementFromPoint(centerX, centerY);
      if (!hit || (hit !== control && !control.contains(hit))) {
        violations.push(
          `control ${name} centre (${round(centerX)}, ${round(centerY)}) hits ${hit ? describe(hit) : "nothing"}`
        );
      }
    }

    return { viewportWidth, layout, violations, hitTargets };
  }, probe);
}

/** Polls until the container satisfies the inner-fit contract, then returns the report. */
export async function expectContained(
  page: Page,
  probe: ContainmentProbe,
  message: string
): Promise<ContainmentReport> {
  await expect
    .poll(async () => (await containmentReport(page, probe)).violations, { message })
    .toEqual([]);
  return containmentReport(page, probe);
}
