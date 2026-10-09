// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { DesktopSnapshot, ScheduledSendItem } from "../domain/types";
import { readyDesktopSnapshotFixture } from "../test/desktopApiFixture";
import { formatScheduledSendTime, scheduledSendCapabilityLabel } from "../app/uiShared";
import { t } from "../i18n/messages";
import { ContextualRightPanel } from "./rightPanel";

type PanelProps = Parameters<typeof ContextualRightPanel>[0];

const ALPHA_ROOM_ID = "!room-alpha:example.invalid";

function scheduledItem(overrides: Partial<ScheduledSendItem>): ScheduledSendItem {
  return {
    scheduled_id: "scheduled-1",
    room_id: ALPHA_ROOM_ID,
    body: "First preview",
    send_at_ms: Date.UTC(2030, 0, 1, 12, 0),
    handle: { kind: "local" },
    ...overrides
  };
}

function openSnapshot(items: ScheduledSendItem[]): DesktopSnapshot {
  const snapshot = readyDesktopSnapshotFixture();
  snapshot.state.ui.scheduled_sends_list = {
    kind: "open",
    scope: { kind: "space", space_id: "!space-alpha:example.invalid" },
    capability: "serverDelayedEvents",
    items
  };
  return snapshot;
}

function panelProps(overrides: Partial<PanelProps> = {}): PanelProps {
  return {
    activeRoom: null,
    activeSpace: null,
    activeSpaceName: "Synthetic Workspace",
    isRecoveryBusy: false,
    mode: "scheduledMessages" as const,
    recoverySecretFilled: false,
    recoverySecretInputRef: { current: null },
    snapshot: openSnapshot([]),
    searchQuery: "",
    searchResults: [],
    onCloseThread: vi.fn(),
    onClosePanel: vi.fn(),
    onOpenThread: vi.fn(),
    onOpenFiles: vi.fn(),
    onRefreshFilesView: vi.fn(),
    onPaginateThreadsList: vi.fn(),
    onOpenRecovery: vi.fn(),
    onRecoverySecretPresenceChange: vi.fn(),
    onReply: vi.fn(),
    onResultSelect: vi.fn(),
    onSubmitRecovery: vi.fn(),
    onAcceptVerification: vi.fn(),
    onBootstrapCrossSigning: vi.fn(),
    onCancelVerification: vi.fn(),
    onConfirmSasVerification: vi.fn(),
    onExportRoomKeys: vi.fn(),
    onImportRoomKeys: vi.fn(),
    onBootstrapSecureBackup: vi.fn(),
    onChangeSecureBackupPassphrase: vi.fn(),
    onEnableKeyBackup: vi.fn(),
    onResetIdentity: vi.fn(),
    onCancelIdentityReset: vi.fn(),
    onSubmitIdentityResetOAuth: vi.fn(),
    onSubmitIdentityResetPassword: vi.fn(),
    onProbeLocalEncryption: vi.fn(),
    onResetLocalData: vi.fn(),
    onThreadComposerDocumentChange: vi.fn(),
    onThreadReplySend: vi.fn(),
    ...overrides
  } as unknown as PanelProps;
}

afterEach(() => {
  cleanup();
});

describe("scheduled messages right panel", () => {
  it("renders an always-visible empty state and closes the panel", () => {
    const onClosePanel = vi.fn();
    render(
      <ContextualRightPanel
        {...panelProps({
          snapshot: openSnapshot([]),
          onClosePanel
        })}
      />
    );

    expect(screen.getByText(t("scheduled.title"))).toBeTruthy();
    expect(screen.getByText(t("scheduled.panelEmpty"))).toBeTruthy();
    expect(screen.queryByRole("list")).toBeNull();

    fireEvent.click(
      screen.getByRole("button", { name: t("action.close", { title: t("scheduled.title") }) })
    );
    expect(onClosePanel).toHaveBeenCalledTimes(1);
  });

  it("renders the Rust-projected order, time, preview, room label and thread flag", () => {
    const older = scheduledItem({
      scheduled_id: "scheduled-1",
      body: "First preview",
      send_at_ms: Date.UTC(2030, 0, 1, 12, 0)
    });
    const threaded = scheduledItem({
      scheduled_id: "scheduled-2",
      body: "Second preview",
      send_at_ms: Date.UTC(2030, 0, 1, 9, 0),
      thread_root_event_id: "$root:example.invalid"
    });
    render(
      <ContextualRightPanel
        {...panelProps({
          // Deliberately not chronological: the panel renders whatever Rust
          // projected and must not re-sort it.
          snapshot: openSnapshot([older, threaded])
        })}
      />
    );

    const items = screen.getAllByRole("listitem");
    expect(items).toHaveLength(2);
    expect(within(items[0]!).getByText("First preview")).toBeTruthy();
    expect(within(items[1]!).getByText("Second preview")).toBeTruthy();
    expect(within(items[0]!).getByText(formatScheduledSendTime(older.send_at_ms))).toBeTruthy();
    expect(within(items[1]!).getByText(formatScheduledSendTime(threaded.send_at_ms))).toBeTruthy();
    expect(within(items[0]!).getByText("synthetic-room")).toBeTruthy();
    expect(within(items[1]!).getByText("synthetic-room")).toBeTruthy();
    expect(within(items[0]!).queryByText(t("scheduled.threadReply"))).toBeNull();
    expect(within(items[1]!).getByText(t("scheduled.threadReply"))).toBeTruthy();
  });

  it("isolates the destination direction and separates the thread marker", () => {
    const threaded = scheduledItem({
      scheduled_id: "scheduled-threaded",
      thread_root_event_id: "$root:example.invalid"
    });
    render(
      <ContextualRightPanel
        {...panelProps({ snapshot: openSnapshot([threaded]) })}
      />
    );

    // Remote/user text: the room label must isolate its direction at the UI
    // boundary so a bidi room name cannot reorder the surrounding metadata.
    const destination = screen.getByText("synthetic-room");
    expect(destination.getAttribute("dir")).toBe("auto");
    expect(destination.className).toContain("scheduled-message-room");

    const context = destination.closest(".scheduled-message-context");
    expect(context).not.toBeNull();
    const separator = context!.querySelector<HTMLElement>(".scheduled-message-separator");
    expect(separator).not.toBeNull();
    expect(separator!.getAttribute("aria-hidden")).toBe("true");
    const thread = context!.querySelector<HTMLElement>(".scheduled-message-thread");
    expect(thread).not.toBeNull();
    // The separator sits between the destination and the thread marker.
    expect(destination.compareDocumentPosition(separator!)).toBe(
      Node.DOCUMENT_POSITION_FOLLOWING
    );
    expect(separator!.compareDocumentPosition(thread!)).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
  });

  it("shows the panel's own projected capability, not a selected room's", () => {
    const snapshot = openSnapshot([scheduledItem({})]);
    snapshot.state.ui.timeline.scheduled_send_capability = "unknown";
    snapshot.state.ui.scheduled_sends_list = {
      kind: "open",
      scope: { kind: "home" },
      capability: "localFallback",
      items: [scheduledItem({})]
    };
    render(<ContextualRightPanel {...panelProps({ snapshot })} />);

    expect(screen.getByText(scheduledSendCapabilityLabel("localFallback"))).toBeTruthy();
    expect(screen.getByText(t("scheduled.localFallbackNotice"))).toBeTruthy();
  });

  it("forwards cancel and reschedule to the account-bound handlers", () => {
    const onCancelScheduledSend = vi.fn();
    const onRescheduleScheduledSend = vi.fn();
    render(
      <ContextualRightPanel
        {...panelProps({
          snapshot: openSnapshot([scheduledItem({})]),
          onCancelScheduledSend,
          onRescheduleScheduledSend
        })}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: t("scheduled.cancel") }));
    expect(onCancelScheduledSend).toHaveBeenCalledWith("scheduled-1");

    fireEvent.click(screen.getByRole("button", { name: t("scheduled.edit") }));
    const body = screen.getByRole("textbox", { name: t("scheduled.bodyInput") });
    body.textContent = "Edited body";
    fireEvent.input(body);
    fireEvent.click(screen.getByRole("button", { name: t("scheduled.save") }));
    expect(onRescheduleScheduledSend).toHaveBeenCalledWith(
      "scheduled-1",
      "Edited body",
      expect.any(Number)
    );
  });
});
