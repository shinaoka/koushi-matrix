// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";

const openExternalHttpUrl = vi.fn(async (_url: string) => undefined);
vi.mock("../../backend/linkMediaRuntime", () => ({
  openExternalHttpUrl: (url: string) => openExternalHttpUrl(url)
}));

import { MigrationSection } from "./MigrationSection";
import type { DesktopApi } from "../../backend/desktopApi";
import type {
  MigrationSources,
  RoomKeyImportState
} from "../../domain/types";

afterEach(() => {
  cleanup();
  openExternalHttpUrl.mockClear();
});

const IDLE_IMPORT: RoomKeyImportState = { kind: "idle" };

function sources(overrides: Partial<MigrationSources["element_desktop"]> = {}): MigrationSources {
  return {
    element_x: { sessions: 0 },
    element_desktop: {
      installed: true,
      running: false,
      import_supported: true,
      import_enabled: true,
      unavailable_reason: null,
      ...overrides
    },
    unsupported_reason: null
  };
}

function renderSection(options: {
  detection?: MigrationSources | "fails";
  importState?: RoomKeyImportState;
  chooseSource?: () => Promise<string | null>;
  importRoomKeys?: (path: string, passphrase: string) => void;
  onCreateSecureBackup?: () => void;
} = {}) {
  const detect = vi.fn(async () => {
    if (options.detection === "fails") {
      throw new Error("detection failed");
    }
    return options.detection ?? sources();
  });
  const importFromElement = vi.fn(async () => ({
    admission: { protocolVersion: 1 as const, admittedGeneration: 7 },
    staged_keys: 12,
    staged_rooms: 3
  }));
  const startOwnUserSas = vi.fn(async () => ({
    protocolVersion: 1 as const,
    admittedGeneration: 7
  }));
  const recoverSecureBackup = vi.fn(async (_secret: string) => ({
    protocolVersion: 1 as const,
    admittedGeneration: 7
  }));
  const api = {
    migrationDetectSources: detect,
    migrationImportElementDesktop: importFromElement,
    startOwnUserSas,
    recoverSecureBackup
  } as unknown as DesktopApi;
  const onChooseRoomKeyImportSource =
    options.chooseSource ?? vi.fn(async () => "/synthetic/element-keys.txt");
  const onImportRoomKeys = options.importRoomKeys ?? vi.fn();
  const onCreateSecureBackup = options.onCreateSecureBackup ?? vi.fn();
  render(
    <MigrationSection
      desktopApi={api}
      roomKeyImport={options.importState ?? IDLE_IMPORT}
      onChooseRoomKeyImportSource={onChooseRoomKeyImportSource}
      onImportRoomKeys={onImportRoomKeys}
      onCreateSecureBackup={onCreateSecureBackup}
    />
  );
  return { detect, importFromElement, startOwnUserSas, recoverSecureBackup, onImportRoomKeys, onCreateSecureBackup };
}

async function settled() {
  // Detection lands in an effect; wait for its line, not for a substring that
  // also appears in route headings.
  await waitFor(() =>
    expect(
      (document.querySelector("[data-migration-detection]") as HTMLElement).textContent
        ?.length ?? 0
    ).toBeGreaterThan(0)
  );
}

describe("Migrate to Koushi (#1871)", () => {
  test("offers the routes in the order that works, with the file import first and marked", async () => {
    renderSection();
    await settled();
    const routes = [...document.querySelectorAll("[data-migration-route]")].map((node) =>
      node.getAttribute("data-migration-route")
    );
    expect(routes).toEqual(["key-file", "recovery-key", "element-desktop", "verify-device", "nothing"]);

    // Route 1 is the one the screen calls the most reliable.
    const keyFile = document.querySelector('[data-migration-route="key-file"]') as HTMLElement;
    expect(within(keyFile).getByText("most reliable")).toBeTruthy();
    expect(within(keyFile).getByRole("form", { name: "1. Import an exported key file" })).toBeTruthy();

    // Verifying from another device is last among the ways to get keys, and it
    // says up front that it can stall.
    const verify = document.querySelector('[data-migration-route="verify-device"]') as HTMLElement;
    expect(within(verify).getByText(/known to stall/i)).toBeTruthy();
    expect(within(verify).getByRole("button", { name: "Go to route 1" })).toBeTruthy();
    expect(within(verify).getByRole("button", { name: "Go to route 2" })).toBeTruthy();
  });

  test("imports an exported key file through the existing import path", async () => {
    const onImportRoomKeys = vi.fn();
    renderSection({ importRoomKeys: onImportRoomKeys });
    await settled();

    const keyFile = document.querySelector('[data-migration-route="key-file"]') as HTMLElement;
    fireEvent.click(within(keyFile).getByRole("button", { name: "Import room keys" }));
    // The heading and the field share their label; the dialog field is the input.
    const passphrase = await waitFor(() => {
      const fields = screen.getAllByLabelText("Room key passphrase");
      const input = fields.find((node) => node.tagName === "INPUT");
      expect(input).toBeTruthy();
      return input as HTMLInputElement;
    });
    fireEvent.change(passphrase, { target: { value: "synthetic-passphrase" } });
    fireEvent.submit(passphrase.closest("form") as HTMLFormElement);
    await waitFor(() =>
      expect(onImportRoomKeys).toHaveBeenCalledWith(
        "/synthetic/element-keys.txt",
        "synthetic-passphrase"
      )
    );
    await waitFor(() => expect(screen.queryAllByLabelText("Room key passphrase").length).toBe(0));
  });

  test("restores from a recovery key without involving another device", async () => {
    const { recoverSecureBackup } = renderSection();
    await settled();

    const recovery = document.querySelector(
      '[data-migration-route="recovery-key"]'
    ) as HTMLElement;
    const secret = within(recovery).getByLabelText(
      "Recovery key or security phrase"
    ) as HTMLInputElement;
    const submit = within(recovery).getByRole("button", { name: "Restore keys" });
    expect((submit as HTMLButtonElement).disabled).toBe(true);

    fireEvent.change(secret, { target: { value: "synthetic-recovery-key" } });
    fireEvent.submit(secret.closest("form") as HTMLFormElement);

    await waitFor(() =>
      expect(recoverSecureBackup).toHaveBeenCalledWith("synthetic-recovery-key")
    );
    // The secret does not stay in the field or anywhere on the screen.
    await waitFor(() => expect(secret.value).toBe(""));
    expect(document.body.textContent).not.toContain("synthetic-recovery-key");
  });

  test("asks before importing from Element Desktop, then reports what it staged", async () => {
    const { importFromElement } = renderSection();
    await settled();

    const element = document.querySelector(
      '[data-migration-route="element-desktop"]'
    ) as HTMLElement;
    fireEvent.click(within(element).getByRole("button", { name: "Import from Element Desktop" }));
    expect(importFromElement).not.toHaveBeenCalled();
    expect(within(element).getByText(/signed in with the same account/i)).toBeTruthy();

    // Cancel first: nothing happens.
    fireEvent.click(within(element).getByRole("button", { name: "Cancel" }));
    expect(importFromElement).not.toHaveBeenCalled();

    fireEvent.click(within(element).getByRole("button", { name: "Import from Element Desktop" }));
    fireEvent.click(
      within(element).getByRole("button", { name: "Import keys from Element Desktop?" })
    );
    await waitFor(() => expect(importFromElement).toHaveBeenCalledTimes(1));
    await waitFor(() =>
      expect(within(element).getByRole("status").textContent).toContain("12 keys")
    );
    expect(within(element).getByRole("status").textContent).toContain("3 rooms");
  });

  test("hides the Element Desktop route when the build has the import turned off", async () => {
    renderSection({
      detection: sources({
        import_enabled: false,
        unavailable_reason: "migration_element_desktop_import_disabled"
      })
    });
    await settled();
    expect(document.querySelector('[data-migration-route="element-desktop"]')).toBeNull();
    expect(document.body.textContent).toContain("turned off in this build");
  });

  test("reports what is on this machine, and that Element X keys stay out of reach", async () => {
    renderSection({
      detection: {
        element_x: { sessions: 2 },
        element_desktop: {
          installed: true,
          running: true,
          import_supported: true,
          import_enabled: true,
          unavailable_reason: null
        },
        unsupported_reason: null
      }
    });
    await settled();
    const detection = document.querySelector("[data-migration-detection]") as HTMLElement;
    expect(detection.textContent).toContain("running on this computer");
    expect(detection.textContent).toContain("Element X data is here too");
  });

  test("says nothing is installed when detection finds no Element Desktop", async () => {
    // The flag stays on: only the missing application hides the route.
    renderSection({
      detection: sources({ installed: false, running: false })
    });
    await waitFor(() =>
      expect(
        (document.querySelector("[data-migration-detection]") as HTMLElement).textContent
      ).toContain("No Element Desktop")
    );
    expect(document.querySelector('[data-migration-route="element-desktop"]')).toBeNull();
  });

  test("survives a detection failure instead of promising a route it cannot see", async () => {
    renderSection({ detection: "fails" });
    await waitFor(() =>
      expect(
        (document.querySelector("[data-migration-detection]") as HTMLElement).textContent
      ).toContain("could not be checked")
    );
    // An unchecked machine is not an offer.
    expect(document.querySelector('[data-migration-route="element-desktop"]')).toBeNull();
  });

  test("starts device verification and can send the reader back to route 1 or 2", async () => {
    const { startOwnUserSas } = renderSection();
    await settled();
    const verify = document.querySelector('[data-migration-route="verify-device"]') as HTMLElement;

    fireEvent.click(within(verify).getByRole("button", { name: "Verify with another device" }));
    await waitFor(() => expect(startOwnUserSas).toHaveBeenCalledTimes(1));

    const recovery = document.querySelector(
      '[data-migration-route="recovery-key"]'
    ) as HTMLElement;
    const secret = within(recovery).getByLabelText("Recovery key or security phrase");
    fireEvent.click(within(verify).getByRole("button", { name: "Go to route 2" }));
    expect(document.activeElement).toBe(secret);

    const keyFileButton = within(
      document.querySelector('[data-migration-route="key-file"]') as HTMLElement
    ).getByRole("button", { name: "Import room keys" });
    fireEvent.click(within(verify).getByRole("button", { name: "Go to route 1" }));
    expect(document.activeElement).toBe(keyFileButton);
  });

  test("states what cannot be recovered and links Element Web for the backup key", async () => {
    const onCreateSecureBackup = vi.fn();
    renderSection({ onCreateSecureBackup });
    await settled();
    const nothing = document.querySelector('[data-migration-route="nothing"]') as HTMLElement;
    expect(nothing.textContent).toContain("stays unreadable");
    expect(nothing.textContent).toContain("future messages only");

    fireEvent.click(within(nothing).getByRole("button", { name: "Create a new secure backup" }));
    expect(onCreateSecureBackup).toHaveBeenCalledTimes(1);

    fireEvent.click(
      within(nothing).getByRole("button", { name: "Open Element Web security settings" })
    );
    await waitFor(() =>
      expect(openExternalHttpUrl).toHaveBeenCalledWith(
        "https://app.element.io/#/settings/security"
      )
    );
  });

  test("shows the command's own import status rather than a second copy of it", async () => {
    renderSection({
      importState: { kind: "imported", request_id: 9, imported_count: 4, total_count: 4 }
    });
    await waitFor(() =>
      expect(screen.getByTestId("migrate-key-file-state").textContent).toContain("4 of 4")
    );
  });
});
