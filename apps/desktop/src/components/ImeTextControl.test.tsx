// @vitest-environment jsdom

import { createRef } from "react";
import { cleanup, createEvent, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  ImeSafeForm,
  ImeTextArea,
  ImeTextField,
  SecureImeTextField
} from "./ImeTextControl";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("IME text controls", () => {
  it.each([
    ["text", (props: { value: string; syncKey: string }) => (
      <ImeTextField aria-label="field" {...props} />
    )],
    ["search", (props: { value: string; syncKey: string }) => (
      <ImeTextField aria-label="field" type="search" {...props} />
    )],
    ["textarea", (props: { value: string; syncKey: string }) => (
      <ImeTextArea aria-label="field" {...props} />
    )]
  ] as const)("keeps %s DOM value and selection across stale composition rerenders", (_kind, field) => {
    const { rerender } = render(field({ value: "before", syncKey: "field-a" }));
    const control = screen.getByLabelText("field") as
      | HTMLInputElement
      | HTMLTextAreaElement;

    fireEvent.compositionStart(control);
    fireEvent.change(control, { target: { value: "日本語変換中" } });
    control.setSelectionRange(3, 5);
    rerender(field({ value: "stale external", syncKey: "field-a" }));

    expect(control.value).toBe("日本語変換中");
    expect([control.selectionStart, control.selectionEnd]).toEqual([3, 5]);
  });

  it("keeps a dirty local value until an external acknowledgement arrives", () => {
    const { rerender } = render(
      <ImeTextField aria-label="field" value="before" syncKey="field-a" />
    );
    const control = screen.getByRole("textbox", { name: "field" }) as HTMLInputElement;

    fireEvent.change(control, { target: { value: "local" } });
    rerender(<ImeTextField aria-label="field" value="before" syncKey="field-a" />);
    expect(control.value).toBe("local");

    rerender(<ImeTextField aria-label="field" value="local" syncKey="field-a" />);
    rerender(<ImeTextField aria-label="field" value="server" syncKey="field-a" />);
    expect(control.value).toBe("server");
  });

  it("forces the next semantic field value when syncKey changes", () => {
    const { rerender } = render(
      <ImeTextField aria-label="field" value="before" syncKey="field-a" />
    );
    const control = screen.getByRole("textbox", { name: "field" }) as HTMLInputElement;
    fireEvent.compositionStart(control);
    fireEvent.change(control, { target: { value: "old composition" } });

    rerender(<ImeTextField aria-label="field" value="next" syncKey="field-b" />);

    expect(control.value).toBe("next");
  });

  it("keeps secure values DOM-only behind a forwarded ref", () => {
    const ref = createRef<HTMLInputElement>();
    render(<SecureImeTextField ref={ref} aria-label="secret" autoComplete="off" />);
    const control = screen.getByLabelText("secret") as HTMLInputElement;

    fireEvent.input(control, { target: { value: "private value" } });

    expect(ref.current).toBe(control);
    expect(ref.current?.value).toBe("private value");
  });

  it("suppresses IME-confirmation submit without preventing the native key default", () => {
    vi.useFakeTimers();
    const onSubmit = vi.fn((event: React.FormEvent<HTMLFormElement>) => event.preventDefault());
    const onKeyDown = vi.fn();
    render(
      <ImeSafeForm aria-label="form" onSubmit={onSubmit}>
        <ImeTextField aria-label="field" onKeyDown={onKeyDown} />
        <button type="submit">Submit</button>
      </ImeSafeForm>
    );
    const form = screen.getByRole("form", { name: "form" });
    const control = screen.getByRole("textbox", { name: "field" });

    fireEvent.compositionStart(control);
    const imeEnter = createEvent.keyDown(control, {
      key: "Enter",
      code: "Enter",
      keyCode: 229,
      isComposing: true
    });
    fireEvent(control, imeEnter);
    fireEvent.submit(form);

    expect(imeEnter.defaultPrevented).toBe(false);
    expect(onKeyDown).not.toHaveBeenCalled();
    expect(onSubmit).not.toHaveBeenCalled();

    fireEvent.compositionEnd(control);
    vi.runAllTimers();
    fireEvent.keyDown(control, { key: "Enter", code: "Enter", keyCode: 13 });
    fireEvent.submit(form);
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });
});
