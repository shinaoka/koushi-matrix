// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";

import {
  HistoryExportDialog,
  historyExportSummary,
  type HistoryExportControls,
  type HistoryExportTarget
} from "./HistoryExportDialog";
import { RoomInfoPanel } from "./RoomInfoPanel";
import { historyExportLabels } from "../domain/historyExportLabels";
import { setActiveLocaleProfile, t } from "../i18n/messages";
import type {
  HistoryExportRangeInput,
  HistoryExportRoom,
  HistoryExportRoomCounts,
  HistoryExportScopeInput,
  HistoryExportStart,
  HistoryExportState,
  RoomSummary
} from "../domain/types";

const room: RoomSummary = {
  room_id: "!history:example.invalid",
  display_name: "Synthetic History",
  display_label: "Synthetic History",
  original_display_label: "Synthetic History",
  avatar: null,
  is_dm: false,
  dm_user_ids: [],
  tags: { favourite: null, low_priority: null },
  parent_space_ids: [],
  dm_space_ids: [],
  is_encrypted: true,
  unread_count: 0
};

const roomTarget: HistoryExportTarget = {
  kind: "room",
  roomId: room.room_id,
  name: "Synthetic History",
  encrypted: true
};
const spaceTarget: HistoryExportTarget = { kind: "space", spaceId: "!lab:example.invalid", name: "Lab" };
const spaceScope = { kind: "space", space_id: "!lab:example.invalid" } as const;
const roomScope = { kind: "room", room_id: room.room_id } as const;

function counts(partial: Partial<HistoryExportRoomCounts> = {}): HistoryExportRoomCounts {
  return {
    fetched_events: 0,
    exported_events: 0,
    undecryptable_events: 0,
    attachments_total: 0,
    attachments_done: 0,
    attachments_failed: 0,
    ...partial
  };
}

function exportRoom(
  id: string,
  phase: HistoryExportRoom["phase"],
  partial: Partial<HistoryExportRoom> = {}
): HistoryExportRoom {
  return {
    room_id: id,
    display_name: `Room ${id}`,
    phase,
    counts: counts(),
    skip_reason: phase === "skipped" ? "notJoined" : null,
    failure_kind: null,
    ...partial
  };
}

const idle: HistoryExportState = { kind: "idle" };

function running(requestId: number, rooms: HistoryExportRoom[], stopRequested = false): HistoryExportState {
  return { kind: "running", request_id: requestId, scope: spaceScope, range: { kind: "allAvailable" }, rooms, stop_requested: stopRequested };
}

function controls(started: HistoryExportStart = { kind: "dismissed" }, retried: HistoryExportStart = started) {
  return {
    loadTimeZone: vi.fn(async () => "Asia/Tokyo"),
    start: vi.fn(
      async (_scope: HistoryExportScopeInput, _range: HistoryExportRangeInput, _title: string, _stem: string) => started
    ),
    stop: vi.fn(async () => undefined),
    retry: vi.fn(async (_requestId: number) => retried)
  } satisfies HistoryExportControls;
}

const submitted = (requestId: number): HistoryExportStart => ({
  kind: "submitted",
  requestId,
  admission: { kind: "admitted" } as never
});

function renderDialog(target: HistoryExportTarget, exportState: HistoryExportState, ports: HistoryExportControls) {
  const onClose = vi.fn();
  const view = render(<HistoryExportDialog target={target} exportState={exportState} controls={ports} onClose={onClose} />);
  return {
    onClose,
    rerender: (next: HistoryExportState) =>
      view.rerender(<HistoryExportDialog target={target} exportState={next} controls={ports} onClose={onClose} />)
  };
}

afterEach(() => {
  cleanup();
  setActiveLocaleProfile("en", "none");
});

describe.each(["en", "ja"] as const)("history export dialog in %s", (locale) => {
  test("a room export sends the displayed civil period, zone, and room name, with every warning", async () => {
    setActiveLocaleProfile(locale, "none");
    const ports = controls();
    renderDialog(roomTarget, idle, ports);
    expect(screen.getByTestId("history-export-plaintext-warning").textContent).toBe(t("historyExport.plaintextWarning"));
    expect(screen.getByTestId("history-export-size-warning").textContent).toBe(t("historyExport.attachments"));
    const period = screen.getByRole("radio", { name: t("historyExport.rangePeriod") });
    await waitFor(() => expect((period as HTMLInputElement).disabled).toBe(false));
    fireEvent.click(period);
    expect(screen.getByTestId("history-export-time-zone").textContent).toBe(
      t("historyExport.timeZone", { timeZone: "Asia/Tokyo" })
    );
    fireEvent.change(screen.getByLabelText(t("historyExport.startDate")), { target: { value: "2026-09-01" } });
    fireEvent.change(screen.getByLabelText(t("historyExport.endDate")), { target: { value: "2026-09-30" } });
    fireEvent.click(screen.getByRole("button", { name: t("historyExport.save") }));
    await waitFor(() => expect(ports.start).toHaveBeenCalledTimes(1));
    expect(ports.start).toHaveBeenCalledWith(
      { kind: "room", roomId: room.room_id },
      { kind: "period", startDate: "2026-09-01", endDate: "2026-09-30", timeZone: "Asia/Tokyo" },
      t("historyExport.folderDialogTitle"),
      "Synthetic History"
    );
  });

  test("a Space export sends the Space scope", async () => {
    setActiveLocaleProfile(locale, "none");
    const ports = controls();
    renderDialog(spaceTarget, idle, ports);
    expect(screen.getByRole("dialog", { name: t("historyExport.spaceTitle") })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: t("historyExport.save") }));
    await waitFor(() => expect(ports.start).toHaveBeenCalledTimes(1));
    expect(ports.start.mock.calls[0][0]).toEqual({ kind: "space", spaceId: "!lab:example.invalid" });
    expect(ports.start.mock.calls[0][1]).toEqual({ kind: "allAvailable" });
  });

  test("page labels come from the catalog with their placeholders intact", () => {
    setActiveLocaleProfile(locale, "none");
    const labels = historyExportLabels();
    expect(labels.lang).toBe(locale);
    expect(labels.edited).toBe(t("historyExport.page.edited"));
    expect(labels.inReplyTo).toContain("{name}");
    expect(labels.stateInvited).toContain("{target}");
    expect(labels.timesInZone).toContain("{timeZone}");
  });
});

test("an unencrypted room shows no plaintext warning but a Space always does", () => {
  renderDialog({ ...roomTarget, encrypted: false }, idle, controls());
  expect(screen.queryByTestId("history-export-plaintext-warning")).toBeNull();
  cleanup();
  renderDialog(spaceTarget, idle, controls());
  expect(screen.getByTestId("history-export-plaintext-warning")).toBeTruthy();
});

test("an end date before the start date blocks the download", async () => {
  const ports = controls();
  renderDialog(roomTarget, idle, ports);
  const period = screen.getByRole("radio", { name: t("historyExport.rangePeriod") });
  await waitFor(() => expect((period as HTMLInputElement).disabled).toBe(false));
  fireEvent.click(period);
  fireEvent.change(screen.getByLabelText(t("historyExport.startDate")), { target: { value: "2026-09-10" } });
  fireEvent.change(screen.getByLabelText(t("historyExport.endDate")), { target: { value: "2026-09-01" } });
  expect(screen.getByRole("alert").textContent).toBe(t("historyExport.invalidPeriod"));
  expect((screen.getByRole("button", { name: t("historyExport.save") }) as HTMLButtonElement).disabled).toBe(true);
});

test("the progress view lists every room with its phase and counts, and Stop is sent once", async () => {
  const ports = controls(submitted(7));
  const view = renderDialog(spaceTarget, idle, ports);
  fireEvent.click(screen.getByRole("button", { name: t("historyExport.save") }));
  await waitFor(() => expect(ports.start).toHaveBeenCalled());
  const rooms = [
    exportRoom("!a", "completed", { counts: counts({ exported_events: 12, attachments_total: 3, attachments_done: 3 }) }),
    exportRoom("!b", "attachments", { counts: counts({ attachments_total: 10, attachments_done: 4 }) }),
    exportRoom("!c", "skipped"),
    exportRoom("!d", "pending"),
    exportRoom("!e", "failed", { failure_kind: "network" })
  ];
  view.rerender(running(7, rooms));
  expect(screen.getByTestId("history-export-state").textContent).toBe(
    t("historyExport.roomsProgress", { done: 3, total: 5 })
  );
  const items = screen.getByTestId("history-export-rooms").querySelectorAll("li");
  expect(items).toHaveLength(5);
  expect(items[0].querySelector(".export-progress-name")?.getAttribute("dir")).toBe("auto");
  expect(items[0].textContent).toContain(t("historyExport.phaseCompleted", { exported: 12, attachments: 3 }));
  expect(items[1].textContent).toContain(t("historyExport.phaseAttachments", { done: 4, total: 10 }));
  expect(items[2].textContent).toContain(t("historyExport.phaseSkipped"));
  expect(items[3].textContent).toContain(t("historyExport.phasePending"));
  expect(items[4].textContent).toContain(t("historyExport.roomFailedNetwork"));
  fireEvent.click(screen.getByRole("button", { name: t("historyExport.stop") }));
  expect(ports.stop).toHaveBeenCalledWith(7);
  view.rerender(running(7, rooms, true));
  expect(screen.getByTestId("history-export-state").textContent).toBe(t("historyExport.stopping"));
  expect((screen.getByRole("button", { name: t("historyExport.stop") }) as HTMLButtonElement).disabled).toBe(true);
});

test("a stopped export offers to continue, which retries its own request", async () => {
  const ports = controls(submitted(7), submitted(8));
  const view = renderDialog(spaceTarget, idle, ports);
  fireEvent.click(screen.getByRole("button", { name: t("historyExport.save") }));
  await waitFor(() => expect(ports.start).toHaveBeenCalled());
  view.rerender({ kind: "stopped", request_id: 7, scope: spaceScope, range: { kind: "allAvailable" }, rooms: [exportRoom("!a", "completed")] });
  expect(screen.getByTestId("history-export-state").textContent).toContain(t("historyExport.stopped"));
  fireEvent.click(screen.getByRole("button", { name: t("historyExport.resume") }));
  await waitFor(() => expect(ports.retry).toHaveBeenCalledWith(7));
  view.rerender(running(8, [exportRoom("!a", "completed"), exportRoom("!b", "fetching")]));
  expect(screen.getByTestId("history-export-state").textContent).toBe(t("historyExport.roomsProgress", { done: 1, total: 2 }));
});

test("a completed export with failed rooms reports them and offers a retry", async () => {
  const ports = controls(submitted(7), submitted(9));
  const view = renderDialog(spaceTarget, idle, ports);
  fireEvent.click(screen.getByRole("button", { name: t("historyExport.save") }));
  await waitFor(() => expect(ports.start).toHaveBeenCalled());
  view.rerender({
    kind: "completed",
    request_id: 7,
    scope: spaceScope,
    range: { kind: "allAvailable" },
    rooms: [
      exportRoom("!a", "completed", { counts: counts({ attachments_total: 2, attachments_done: 2, attachments_failed: 1, undecryptable_events: 4 }) }),
      exportRoom("!b", "failed", { failure_kind: "sdk" })
    ]
  });
  expect(screen.getByTestId("history-export-state").textContent).toBe(t("historyExport.completedWithFailures", { count: 1 }));
  expect(screen.getByTestId("history-export-rooms").textContent).toContain(t("historyExport.attachmentsFailed", { count: 1 }));
  expect(screen.getByTestId("history-export-rooms").textContent).toContain(t("historyExport.undecryptable", { count: 4 }));
  fireEvent.click(screen.getByRole("button", { name: t("historyExport.retryFailed") }));
  await waitFor(() => expect(ports.retry).toHaveBeenCalledWith(7));
});

test("a failed export shows its Rust failure kind and is never reported as finished", async () => {
  const ports = controls(submitted(7));
  const view = renderDialog(spaceTarget, idle, ports);
  fireEvent.click(screen.getByRole("button", { name: t("historyExport.save") }));
  await waitFor(() => expect(ports.start).toHaveBeenCalled());
  view.rerender({ kind: "failed", request_id: 7, scope: spaceScope, range: { kind: "allAvailable" }, rooms: [], failure_kind: "manifestMismatch" });
  const state = screen.getByTestId("history-export-state").textContent ?? "";
  expect(state).toContain(t("historyExport.failed"));
  expect(state).toContain(t("historyExport.failedManifestMismatch"));
  expect(state).not.toContain(t("historyExport.completed"));
});

test("a settlement of an earlier request is not shown as this dialog's result", async () => {
  const ports = controls(submitted(8));
  const view = renderDialog(roomTarget, idle, ports);
  fireEvent.click(screen.getByRole("button", { name: t("historyExport.save") }));
  await waitFor(() => expect(ports.start).toHaveBeenCalled());
  view.rerender({ kind: "completed", request_id: 3, scope: roomScope, range: { kind: "allAvailable" }, rooms: [] });
  expect(screen.getByRole("alert").textContent).toBe(t("historyExport.notStarted"));
});

test("a rejected or failed start is reported without a result", async () => {
  const ports = controls();
  ports.start.mockRejectedValueOnce(new Error("adapter"));
  renderDialog(roomTarget, idle, ports);
  fireEvent.click(screen.getByRole("button", { name: t("historyExport.save") }));
  await waitFor(() => expect(screen.getByRole("alert").textContent).toBe(t("historyExport.notStarted")));
});

test("a dismissed folder dialog keeps the form without an error", async () => {
  const ports = controls({ kind: "dismissed" });
  renderDialog(roomTarget, idle, ports);
  fireEvent.click(screen.getByRole("button", { name: t("historyExport.save") }));
  await waitFor(() => expect(ports.start).toHaveBeenCalled());
  await act(async () => undefined);
  expect(screen.queryByRole("alert")).toBeNull();
  expect(screen.getByRole("button", { name: t("historyExport.save") })).toBeTruthy();
});

test("another export in flight blocks a new one", () => {
  renderDialog(roomTarget, running(4, [exportRoom("!a", "fetching")]), controls());
  expect(screen.getByText(t("historyExport.busy"))).toBeTruthy();
  expect((screen.getByRole("button", { name: t("historyExport.save") }) as HTMLButtonElement).disabled).toBe(true);
});

test("the summary follows only the target's export", () => {
  const state = running(4, [exportRoom("!a", "completed"), exportRoom("!b", "fetching")]);
  expect(historyExportSummary(state, spaceTarget)).toEqual([t("historyExport.summaryRunning", { done: 1, total: 2 })]);
  expect(historyExportSummary(state, roomTarget)).toEqual([]);
  expect(historyExportSummary(idle, spaceTarget)).toEqual([]);
  expect(
    historyExportSummary({ kind: "preparing", request_id: 1, scope: spaceScope, range: { kind: "allAvailable" }, stop_requested: false }, spaceTarget)
  ).toEqual([t("historyExport.preparing")]);
});

test("Room info shows the section and its summary", () => {
  render(
    <RoomInfoPanel
      room={room}
      roomNotificationSettings={undefined}
      spaces={[]}
      historyExport={{ kind: "completed", request_id: 2, scope: roomScope, range: { kind: "allAvailable" }, rooms: [] }}
      historyExportControls={controls()}
    />
  );
  expect(screen.getByRole("region", { name: t("historyExport.section") })).toBeTruthy();
  expect(screen.getByTestId("history-export-summary").textContent).toBe(t("historyExport.completed"));
});

test("reopening the dialog after the export settled shows its result and Retry, then a new download", async () => {
  const ports = controls(submitted(11), submitted(12));
  renderDialog(spaceTarget, {
    kind: "completed",
    request_id: 7,
    scope: spaceScope,
    range: { kind: "allAvailable" },
    rooms: [exportRoom("!a", "completed"), exportRoom("!b", "failed", { failure_kind: "network" })]
  }, ports);
  expect(screen.getByTestId("history-export-state").textContent).toBe(
    t("historyExport.completedWithFailures", { count: 1 })
  );
  expect(screen.getByTestId("history-export-rooms").querySelectorAll("li")).toHaveLength(2);
  fireEvent.click(screen.getByRole("button", { name: t("historyExport.retryFailed") }));
  await waitFor(() => expect(ports.retry).toHaveBeenCalledWith(7));
  cleanup();
  renderDialog(spaceTarget, { kind: "stopped", request_id: 7, scope: spaceScope, range: { kind: "allAvailable" }, rooms: [] }, ports);
  fireEvent.click(screen.getByRole("button", { name: t("historyExport.again") }));
  expect(screen.getByRole("button", { name: t("historyExport.save") })).toBeTruthy();
});

test("another target's settled export does not replace this dialog's form", () => {
  renderDialog(roomTarget, { kind: "completed", request_id: 7, scope: spaceScope, range: { kind: "allAvailable" }, rooms: [] }, controls());
  expect(screen.getByRole("button", { name: t("historyExport.save") })).toBeTruthy();
});
