// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  CreateEntityDialog,
  type CreateRoomDialogOptions,
  DiagnosticDialog
} from "./dialogs";

afterEach(cleanup);

describe("DiagnosticDialog", () => {
  it("shows copyable diagnostics", () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, { clipboard: { writeText } });

    render(
      <DiagnosticDialog
        report={"Koushi diagnostics\nDownloading messages from 1 room(s)"}
        onClose={vi.fn()}
      />
    );

    expect(screen.getByRole("dialog", { name: "Diagnostics" })).toBeTruthy();
    expect(screen.getByText(/Downloading messages from 1 room/)).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Copy diagnostics" }));

    expect(writeText).toHaveBeenCalledWith(
      "Koushi diagnostics\nDownloading messages from 1 room(s)"
    );
  });
});

describe("CreateEntityDialog", () => {
  it("collects private space-room options without exposing encryption for public rooms", () => {
    const onSubmit = vi.fn();
    const onRoomOptionsChange = vi.fn();

    function ControlledDialog() {
      const [roomOptions, setRoomOptions] = useState<CreateRoomDialogOptions>({
        aliasLocalpart: "",
        encrypted: true,
        topic: "",
        visibility: "private"
      });
      return (
        <CreateEntityDialog
          activeSpaceName="Synthetic Workspace"
          isBusy={false}
          kind="room"
          roomOptions={roomOptions}
          value="Ops Room"
          onCancel={vi.fn()}
          onRoomOptionsChange={(next) => {
            setRoomOptions(next);
            onRoomOptionsChange(next);
          }}
          onSubmit={onSubmit}
          onValueChange={vi.fn()}
        />
      );
    }

    render(<ControlledDialog />);

    expect(
      (screen.getByRole("radio", { name: "Private room" }) as HTMLInputElement).checked
    ).toBe(true);
    expect(screen.getByText("Standard room in Synthetic Workspace")).toBeTruthy();
    fireEvent.change(screen.getByRole("textbox", { name: "Topic" }), {
      target: { value: "Deployment notes" }
    });
    expect(onRoomOptionsChange).toHaveBeenLastCalledWith({
      aliasLocalpart: "",
      encrypted: true,
      topic: "Deployment notes",
      visibility: "private"
    });

    fireEvent.click(screen.getByRole("radio", { name: "Public room" }));
    expect(screen.queryByRole("checkbox", { name: "Encrypted room" })).toBeNull();
    fireEvent.change(screen.getByRole("textbox", { name: "Room address" }), {
      target: { value: "ops-room" }
    });
    expect(onRoomOptionsChange).toHaveBeenLastCalledWith({
      aliasLocalpart: "ops-room",
      encrypted: false,
      topic: "Deployment notes",
      visibility: "public"
    });

    fireEvent.click(screen.getByRole("button", { name: "Submit create room" }));
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });
});
