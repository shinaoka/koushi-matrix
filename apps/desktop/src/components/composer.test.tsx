// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { MentionCandidate } from "../domain/projectionTypes";
import { Composer, ThreadComposer } from "./composer";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("Composer", () => {
  const mentionCandidates: MentionCandidate[] = [
    {
      key: "@alice:example.invalid",
      label: "Alice",
      searchText: "alice @alice:example.invalid",
      target: {
        kind: "user",
        user_id: "@alice:example.invalid",
        display_label: "Alice"
      }
    },
    {
      key: "@bob:example.invalid",
      label: "Bob",
      searchText: "bob @bob:example.invalid",
      target: {
        kind: "user",
        user_id: "@bob:example.invalid",
        display_label: "Bob"
      }
    },
    {
      key: "roomMention",
      label: "@room",
      searchText: "room @room notify the whole room",
      target: {
        kind: "roomMention",
        display_label: "room"
      }
    }
  ];

  it("keeps the live conversion DOM value and selection across parent rerenders", () => {
    const props = {
      composerMode: { kind: "plain" as const },
      isSending: false,
      roomName: "Direct room",
      value: "before",
      onCancelReply: () => undefined,
      onSend: vi.fn(),
      onValueChange: vi.fn()
    };
    const { container, rerender } = render(<Composer {...props} />);
    const textarea = container.querySelector("textarea")!;

    fireEvent.compositionStart(textarea);
    fireEvent.change(textarea, { target: { value: "日本語変換中" } });
    textarea.setSelectionRange(3, 5);
    rerender(<Composer {...props} value="stale parent draft" roomName="Renamed room" />);

    expect(textarea.value).toBe("日本語変換中");
    expect([textarea.selectionStart, textarea.selectionEnd]).toEqual([3, 5]);
  });

  it("gives the thread textarea the same live conversion ownership", () => {
    const props = {
      canEdit: true,
      draft: "before",
      isSending: false,
      resolveComposerKeyAction: vi.fn(async () => "noop" as const),
      onDraftChange: vi.fn(),
      onSend: vi.fn()
    };
    const { rerender } = render(<ThreadComposer {...props} />);
    const textarea = screen.getByRole("textbox", { name: /thread/i }) as HTMLTextAreaElement;

    fireEvent.compositionStart(textarea);
    fireEvent.change(textarea, { target: { value: "日本語変換中" } });
    textarea.setSelectionRange(3, 5);
    rerender(<ThreadComposer {...props} draft="stale parent draft" isSending />);

    expect(textarea.value).toBe("日本語変換中");
    expect([textarea.selectionStart, textarea.selectionEnd]).toEqual([3, 5]);
  });

  it("does not submit while an IME composition is being confirmed with Enter", async () => {
    const onSend = vi.fn();
    const resolveComposerKeyAction = vi.fn(async () => "send" as const);

    const { container } = render(
      <Composer
        composerMode={{ kind: "plain" }}
        isSending={false}
        roomName="Direct room"
        value="日本語"
        resolveComposerKeyAction={resolveComposerKeyAction}
        onCancelReply={() => undefined}
        onSend={onSend}
        onValueChange={() => undefined}
      />
    );

    const textarea = container.querySelector("textarea");
    expect(textarea).not.toBeNull();

    fireEvent.compositionStart(textarea!);
    fireEvent.keyDown(textarea!, {
      key: "Enter",
      code: "Enter",
      keyCode: 13
    });

    await Promise.resolve();

    expect(onSend).not.toHaveBeenCalled();
    expect(resolveComposerKeyAction).not.toHaveBeenCalled();
  });

  it("does not let composition A's deferred end clear composition B", async () => {
    vi.useFakeTimers();
    const onSend = vi.fn();
    const resolveComposerKeyAction = vi.fn(async () => "send" as const);
    const { container } = render(
      <Composer
        composerMode={{ kind: "plain" }}
        isSending={false}
        roomName="Direct room"
        value="日本語"
        resolveComposerKeyAction={resolveComposerKeyAction}
        onCancelReply={() => undefined}
        onSend={onSend}
        onValueChange={() => undefined}
      />
    );
    const textarea = container.querySelector("textarea")!;

    fireEvent.compositionStart(textarea);
    fireEvent.compositionEnd(textarea);
    fireEvent.compositionStart(textarea);
    vi.runAllTimers();
    fireEvent.keyDown(textarea, { key: "Enter", code: "Enter", keyCode: 13 });
    await Promise.resolve();

    expect(resolveComposerKeyAction).not.toHaveBeenCalled();
    expect(onSend).not.toHaveBeenCalled();
  });

  it("finishes old DOM ownership and syncs a switched draft exactly once", async () => {
    vi.useFakeTimers();
    const onSend = vi.fn();
    const onValueChange = vi.fn();
    const resolveComposerKeyAction = vi.fn(async () => "send" as const);
    const props = {
      composerMode: { kind: "plain" as const },
      isSending: false,
      roomName: "Room A",
      value: "old draft",
      draftKey: "room-a",
      resolveComposerKeyAction,
      onCancelReply: () => undefined,
      onSend,
      onValueChange
    };
    const { container, rerender } = render(<Composer {...props} />);
    const textarea = container.querySelector("textarea")!;
    fireEvent.compositionStart(textarea);
    fireEvent.change(textarea, { target: { value: "旧変換中" } });
    fireEvent.compositionEnd(textarea);

    const valueDescriptor = Object.getOwnPropertyDescriptor(
      HTMLTextAreaElement.prototype,
      "value"
    )!;
    let imperativeWrites = 0;
    Object.defineProperty(textarea, "value", {
      configurable: true,
      get: () => valueDescriptor.get!.call(textarea),
      set: (value: string) => {
        imperativeWrites += 1;
        valueDescriptor.set!.call(textarea, value);
      }
    });
    rerender(
      <Composer
        {...props}
        draftKey="room-b"
        roomName="Room B"
        value="new room draft"
      />
    );

    expect(textarea.value).toBe("new room draft");
    expect([textarea.selectionStart, textarea.selectionEnd]).toEqual([14, 14]);
    expect(imperativeWrites).toBe(1);
    expect(onValueChange).toHaveBeenCalledTimes(1);

    vi.runAllTimers();
    fireEvent.keyDown(textarea, { key: "Enter", code: "Enter", keyCode: 13 });
    await Promise.resolve();
    expect(resolveComposerKeyAction).toHaveBeenCalledTimes(1);
    expect(onSend).toHaveBeenCalledTimes(1);
    expect(imperativeWrites).toBe(1);
    expect(onValueChange).toHaveBeenCalledTimes(1);
  });

  it("keeps typed text local and sends it before parent state catches up", () => {
    const onSend = vi.fn();
    const onValueChange = vi.fn();

    const { container } = render(
      <Composer
        composerMode={{ kind: "plain" }}
        isSending={false}
        roomName="Direct room"
        value=""
        onCancelReply={() => undefined}
        onSend={onSend}
        onValueChange={onValueChange}
      />
    );

    const textarea = container.querySelector("textarea");
    expect(textarea).not.toBeNull();
    fireEvent.change(textarea!, {
      target: { value: "pasted text that should appear immediately" }
    });

    expect(textarea!.value).toBe("pasted text that should appear immediately");
    expect(onValueChange).toHaveBeenCalledWith("pasted text that should appear immediately");

    fireEvent.click(screen.getByLabelText("Send"));

    expect(onSend).toHaveBeenCalledWith("pasted text that should appear immediately");
  });

  it("moves the active mention row with arrows and accepts it with Tab", () => {
    const onMentionIntentChange = vi.fn();
    const onValueChange = vi.fn();

    const { container } = render(
      <Composer
        composerMode={{ kind: "plain" }}
        isSending={false}
        mentionCandidates={mentionCandidates}
        roomName="Direct room"
        value="@"
        onCancelReply={() => undefined}
        onMentionIntentChange={onMentionIntentChange}
        onSend={() => undefined}
        onValueChange={onValueChange}
      />
    );

    const textarea = container.querySelector("textarea");
    expect(textarea).not.toBeNull();

    fireEvent.keyDown(textarea!, { key: "ArrowDown", code: "ArrowDown" });
    expect(
      screen.getByRole("option", { name: "Bob @bob:example.invalid" }).getAttribute("aria-selected")
    ).toBe("true");

    fireEvent.keyDown(textarea!, { key: "Tab", code: "Tab" });

    expect(onValueChange).toHaveBeenLastCalledWith("@Bob ");
    expect(onMentionIntentChange).toHaveBeenCalledWith({
      targets: [
        {
          kind: "user",
          user_id: "@bob:example.invalid",
          display_label: "Bob"
        }
      ]
    });
  });

  it("closes mention suggestions on Escape until the query changes", async () => {
    const resolveComposerKeyAction = vi.fn(async () => "closeAutocomplete" as const);
    const { container } = render(
      <Composer
        composerMode={{ kind: "plain" }}
        isSending={false}
        mentionCandidates={mentionCandidates}
        roomName="Direct room"
        value="@a"
        resolveComposerKeyAction={resolveComposerKeyAction}
        onCancelReply={() => undefined}
        onSend={() => undefined}
        onValueChange={() => undefined}
      />
    );

    const textarea = container.querySelector("textarea");
    expect(textarea).not.toBeNull();
    expect(screen.getByRole("listbox", { name: "Mention suggestions" })).toBeTruthy();

    fireEvent.keyDown(textarea!, { key: "Escape", code: "Escape" });
    await waitFor(() =>
      expect(screen.queryByRole("listbox", { name: "Mention suggestions" })).toBeNull()
    );

    fireEvent.change(textarea!, { target: { value: "@al" } });
    expect(screen.getByRole("listbox", { name: "Mention suggestions" })).toBeTruthy();
  });

  it("renders users and room notification as sectioned mention suggestions", () => {
    render(
      <Composer
        composerMode={{ kind: "plain" }}
        isSending={false}
        mentionCandidates={mentionCandidates}
        roomName="Direct room"
        value="@room"
        onCancelReply={() => undefined}
        onSend={() => undefined}
        onValueChange={() => undefined}
      />
    );

    expect(screen.getByText("Room Notification")).toBeTruthy();
    expect(
      screen.getByRole("option", { name: "@room Notify the whole room" }).getAttribute("aria-selected")
    ).toBe("true");
  });
});
