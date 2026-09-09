// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { documentFromText } from "../domain/composerDocument";
import { t } from "../i18n/messages";
import { Composer } from "./composer";
import { inlineMentionEditorSelection, setInlineMentionEditorSelection } from "./ImeTextControl";

afterEach(cleanup);

describe("mid-sentence mention insertion", () => {
  it.each(["main", "thread"] as const)("preserves both sides and the caret in %s", (surface) => {
    const prefix = "確認をお願いします。";
    const suffix = "続きの文章";
    const onDocumentChange = vi.fn();
    const onSend = vi.fn();
    render(
      <Composer
        surface={surface}
        composerMode={{ kind: "plain" }}
        document={documentFromText(prefix + suffix)}
        isSending={false}
        roomName="Synthetic room"
        mentionCandidates={[{
          key: "@member:example.invalid",
          label: "Member 1",
          target: { kind: "user", user_id: "@member:example.invalid", display_label: "Member 1" }
        }]}
        onCancelReply={() => undefined}
        onDocumentChange={onDocumentChange}
        onSend={onSend}
      />
    );
    const editor = screen.getByRole("textbox", { name: t("composer.messageComposer") }) as HTMLDivElement;
    act(() => {
      editor.focus();
      setInlineMentionEditorSelection(editor, prefix.length, prefix.length);
    });
    fireEvent(editor, new InputEvent("beforeinput", {
      bubbles: true, cancelable: true, inputType: "insertText", data: "@mem"
    }));
    const option = screen.getByRole("option", { name: /Member 1/ });

    fireEvent.compositionStart(editor);
    fireEvent.keyDown(editor, { key: "Enter", code: "Enter", isComposing: true, keyCode: 229 });
    expect(onSend).not.toHaveBeenCalled();
    expect(onDocumentChange.mock.lastCall?.[0].inlines).toEqual([
      { kind: "text", text: prefix + "@mem" + suffix }
    ]);
    fireEvent.compositionEnd(editor);
    fireEvent.click(option);

    expect(onDocumentChange.mock.lastCall?.[0].inlines).toEqual([
      { kind: "text", text: prefix },
      { kind: "mention", target: { kind: "user", user_id: "@member:example.invalid", display_label: "Member 1" }, display_label: "Member 1" },
      { kind: "text", text: " " + suffix }
    ]);
    expect(inlineMentionEditorSelection(editor)).toEqual({ start: prefix.length + 2, end: prefix.length + 2 });
  });
});
