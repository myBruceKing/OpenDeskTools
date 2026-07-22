// @vitest-environment jsdom

import { act, createRef, useState, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DialogShell } from "../../src/components/primitives/Dialog";
import {
  ShortcutCaptureField,
  type ShortcutCaptureFieldHandle
} from "../../src/components/primitives/Field";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

type CloseMethod = "escape" | "cancel" | "close";

function DialogHarness({ onShortcutChange = () => undefined }: { onShortcutChange?: (value: string) => void }) {
  const [open, setOpen] = useState(false);
  const [shortcut, setShortcut] = useState("F1");
  const close = () => setOpen(false);
  const updateShortcut = (value: string) => {
    onShortcutChange(value);
    setShortcut(value);
  };

  return (
    <>
      <button type="button" onClick={() => setOpen(true)}>
        打开编辑
      </button>
      <DialogShell
        open={open}
        title="编辑截图快捷键"
        onClose={close}
        footer={
          <>
            <button type="button" onClick={close}>取消</button>
            <button type="button">保存</button>
          </>
        }
      >
        <ShortcutCaptureField
          autoFocus
          value={shortcut}
          label="截图快捷键"
          onChange={updateShortcut}
        />
      </DialogShell>
    </>
  );
}

describe("dialog keyboard behavior", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
    document.body.replaceChildren();
    vi.restoreAllMocks();
  });

  async function render(ui: ReactNode) {
    await act(async () => root.render(ui));
  }

  async function openDialog() {
    const trigger = getButton("打开编辑");
    trigger.focus();
    await act(async () => trigger.click());
    await flushAnimationFrame();
    return trigger;
  }

  function getButton(name: string) {
    const button = Array.from(document.querySelectorAll("button")).find(
      (candidate) => candidate.textContent === name || candidate.getAttribute("aria-label") === name
    );
    if (!(button instanceof HTMLButtonElement)) {
      throw new Error(`Button not found: ${name}`);
    }
    return button;
  }

  it("keeps Tab and Shift+Tab inside the modal and removes the background from navigation", async () => {
    await render(<DialogHarness />);
    await openDialog();

    const dialog = document.querySelector<HTMLElement>("[role='dialog']");
    expect(dialog).not.toBeNull();
    expect(host.inert).toBe(true);
    expect(host.getAttribute("aria-hidden")).toBe("true");

    const close = getButton("关闭弹窗");
    const save = getButton("保存");

    save.focus();
    dispatchKey(save, "Tab");
    expect(document.activeElement).toBe(close);

    close.focus();
    dispatchKey(close, "Tab", { shiftKey: true });
    expect(document.activeElement).toBe(save);
  });

  it.each<CloseMethod>(["escape", "cancel", "close"])(
    "restores trigger focus after closing through %s",
    async (method) => {
      const onShortcutChange = vi.fn();
      await render(<DialogHarness onShortcutChange={onShortcutChange} />);
      const trigger = await openDialog();

      if (method === "escape") {
        const capture = document.querySelector<HTMLElement>("[role='group']");
        expect(capture).not.toBeNull();
        dispatchKey(capture!, "Escape");
        expect(onShortcutChange).not.toHaveBeenCalled();
      } else {
        await act(async () => getButton(method === "cancel" ? "取消" : "关闭弹窗").click());
      }

      expect(document.querySelector("[role='dialog']")).toBeNull();
      expect(document.activeElement).toBe(trigger);
      expect(host.inert).toBe(false);
      expect(host.hasAttribute("aria-hidden")).toBe(false);
    }
  );

  it.each([
    ["Enter", "Enter"],
    ["Space", " "]
  ])("does not record %s from the nested delete button", async (_label, key) => {
    const onShortcutChange = vi.fn();
    await render(<DialogHarness onShortcutChange={onShortcutChange} />);
    await openDialog();

    const remove = getButton("删除 F1");
    dispatchKey(remove, key);
    dispatchKey(remove, key, {}, "keyup");
    expect(onShortcutChange).not.toHaveBeenCalled();

    await act(async () => remove.click());
    expect(onShortcutChange).toHaveBeenCalledOnce();
    expect(onShortcutChange).toHaveBeenCalledWith("");
  });

  it("consumes pending modifiers when a native token arrives before modifier keyup", async () => {
    const onAppendToken = vi.fn();
    const fieldRef = createRef<ShortcutCaptureFieldHandle>();
    await render(
      <ShortcutCaptureField
        ref={fieldRef}
        value=""
        label="截图快捷键"
        onChange={vi.fn()}
        onAppendToken={onAppendToken}
      />
    );

    const capture = document.querySelector<HTMLElement>("[role='group']");
    expect(capture).not.toBeNull();
    dispatchKey(capture!, "Shift", { shiftKey: true });
    expect(document.body.textContent).toContain("Shift");

    act(() => fieldRef.current?.acceptNativeToken("Shift+Win+S"));
    dispatchKey(capture!, "Shift", {}, "keyup");

    expect(onAppendToken).toHaveBeenCalledTimes(1);
    expect(onAppendToken).toHaveBeenCalledWith("Shift+Win+S");
  });

  it("lets the complete Alt+F4 lifecycle bubble without recording or preventing it", async () => {
    const onAppendToken = vi.fn();
    const bubbledEvents: string[] = [];
    await render(
      <div
        onKeyDown={(event) => bubbledEvents.push(`down:${event.key}`)}
        onKeyUp={(event) => bubbledEvents.push(`up:${event.key}`)}
      >
        <ShortcutCaptureField
          value=""
          label="截图快捷键"
          onChange={vi.fn()}
          onAppendToken={onAppendToken}
        />
      </div>
    );

    const capture = document.querySelector<HTMLElement>("[role='group']")!;
    const events = [
      createKeyboardEvent("keydown", "Alt", { altKey: true }),
      createKeyboardEvent("keydown", "f4", { altKey: true }),
      createKeyboardEvent("keyup", "F4", { altKey: true }),
      createKeyboardEvent("keyup", "Alt")
    ];
    for (const event of events) {
      act(() => capture.dispatchEvent(event));
    }

    const altReleasedFirstEvents = [
      createKeyboardEvent("keydown", "Alt", { altKey: true }),
      createKeyboardEvent("keydown", "F4", { altKey: true }),
      createKeyboardEvent("keyup", "Alt"),
      createKeyboardEvent("keyup", "f4")
    ];
    for (const event of altReleasedFirstEvents) {
      act(() => capture.dispatchEvent(event));
    }

    expect([...events, ...altReleasedFirstEvents].every((event) => !event.defaultPrevented)).toBe(true);
    expect(bubbledEvents).toEqual([
      "down:Alt",
      "down:f4",
      "up:F4",
      "up:Alt",
      "down:Alt",
      "down:F4",
      "up:Alt",
      "up:f4"
    ]);
    expect(onAppendToken).not.toHaveBeenCalled();
    expect(document.body.textContent).not.toContain("Alt");
  });

  it("continues recording ordinary Alt plus letter combinations", async () => {
    const onAppendToken = vi.fn();
    await render(
      <ShortcutCaptureField
        value=""
        label="截图快捷键"
        onChange={vi.fn()}
        onAppendToken={onAppendToken}
      />
    );

    const capture = document.querySelector<HTMLElement>("[role='group']")!;
    const altDown = createKeyboardEvent("keydown", "Alt", { altKey: true });
    const letterDown = createKeyboardEvent("keydown", "k", { altKey: true });
    act(() => capture.dispatchEvent(altDown));
    act(() => capture.dispatchEvent(letterDown));
    act(() => capture.dispatchEvent(createKeyboardEvent("keyup", "k", { altKey: true })));
    act(() => capture.dispatchEvent(createKeyboardEvent("keyup", "Alt")));

    expect(altDown.defaultPrevented).toBe(false);
    expect(letterDown.defaultPrevented).toBe(true);
    expect(onAppendToken).toHaveBeenCalledOnce();
    expect(onAppendToken).toHaveBeenCalledWith("Alt+K");
  });

  it("records the physical key left of Digit1 as Backquote regardless of its text glyph", async () => {
    const onAppendToken = vi.fn();
    await render(
      <ShortcutCaptureField
        value=""
        label="截图快捷键"
        onChange={vi.fn()}
        onAppendToken={onAppendToken}
      />
    );

    const capture = document.querySelector<HTMLElement>("[role='group']")!;
    dispatchKey(capture, "~", { code: "Backquote", shiftKey: true });

    expect(onAppendToken).toHaveBeenCalledWith("Shift+Backquote");
  });

  it.each([
    ["Alt plus top-row digit", "2", { code: "Digit2", altKey: true }, "Alt+2"],
    ["shifted top-row digit", "@", { code: "Digit2", ctrlKey: true, shiftKey: true }, "Ctrl+Shift+2"],
    ["punctuation", "_", { code: "Minus", ctrlKey: true, shiftKey: true }, "Ctrl+Shift+Minus"],
    ["arrow", "ArrowUp", { code: "ArrowUp", ctrlKey: true }, "Ctrl+ArrowUp"],
    ["numpad", "ArrowDown", { code: "Numpad2", ctrlKey: true }, "Ctrl+Numpad2"],
    ["media", "AudioVolumeUp", { code: "AudioVolumeUp", ctrlKey: true }, "Ctrl+AudioVolumeUp"],
    ["modified edit key", "Backspace", { code: "Backspace", ctrlKey: true }, "Ctrl+Backspace"]
  ])("records %s by physical key identity", async (_label, key, init, expected) => {
    const onAppendToken = vi.fn();
    await render(
      <ShortcutCaptureField
        value=""
        label="截图快捷键"
        onChange={vi.fn()}
        onAppendToken={onAppendToken}
      />
    );

    const capture = document.querySelector<HTMLElement>("[role='group']")!;
    dispatchKey(capture, key, init);

    expect(onAppendToken).toHaveBeenCalledWith(expected);
  });

  it("clears an interrupted Alt+F4 lifecycle on blur before the missing keyup events", async () => {
    const onAppendToken = vi.fn();
    await render(
      <>
        <ShortcutCaptureField
          value=""
          label="截图快捷键"
          onChange={vi.fn()}
          onAppendToken={onAppendToken}
        />
        <button type="button">外部按钮</button>
      </>
    );

    const capture = document.querySelector<HTMLElement>("[role='group']")!;
    const outside = getButton("外部按钮");
    await act(async () => capture.focus());
    act(() => capture.dispatchEvent(createKeyboardEvent("keydown", "Alt", { altKey: true })));
    act(() => capture.dispatchEvent(createKeyboardEvent("keydown", "F4", { altKey: true })));

    await act(async () => outside.focus());
    await act(async () => capture.focus());
    act(() => capture.dispatchEvent(createKeyboardEvent("keydown", "Alt", { altKey: true })));
    act(() => capture.dispatchEvent(createKeyboardEvent("keyup", "Alt")));

    expect(onAppendToken).not.toHaveBeenCalled();
  });

  it("removes ordinary pending modifier UI when the capture field blurs", async () => {
    await render(
      <>
        <ShortcutCaptureField
          value=""
          label="截图快捷键"
          onChange={vi.fn()}
          onAppendToken={vi.fn()}
        />
        <button type="button">外部按钮</button>
      </>
    );

    const capture = document.querySelector<HTMLElement>("[role='group']")!;
    await act(async () => capture.focus());
    act(() => capture.dispatchEvent(createKeyboardEvent("keydown", "Control", { ctrlKey: true })));
    expect(document.body.textContent).toContain("Ctrl");

    await act(async () => getButton("外部按钮").focus());
    expect(document.body.textContent).not.toContain("Ctrl");
    expect(document.body.textContent).toContain("按下快捷键");
  });
});

function dispatchKey(
  target: HTMLElement,
  key: string,
  init: KeyboardEventInit = {},
  type: "keydown" | "keyup" = "keydown"
) {
  act(() => {
    target.dispatchEvent(new KeyboardEvent(type, { key, bubbles: true, cancelable: true, ...init }));
  });
}

async function flushAnimationFrame() {
  await act(async () => {
    await new Promise<void>((resolve) => window.requestAnimationFrame(() => resolve()));
  });
}

function createKeyboardEvent(
  type: "keydown" | "keyup",
  key: string,
  init: KeyboardEventInit = {}
) {
  return new KeyboardEvent(type, { key, bubbles: true, cancelable: true, ...init });
}
