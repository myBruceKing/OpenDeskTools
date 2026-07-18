// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  stopCapture: vi.fn(async () => true),
  startCapture: vi.fn(),
  closeEditor: vi.fn(),
  saveEditor: vi.fn(async () => undefined),
  setBinding: vi.fn(),
  appendBindingToken: vi.fn(),
  setForceOverrideSystem: vi.fn(),
  openEditor: vi.fn()
}));

vi.mock("../../src/app/useHotkeyCaptureSession", () => ({
  useHotkeyCaptureSession: () => ({
    status: "active",
    message: null,
    start: mocks.startCapture,
    stop: mocks.stopCapture
  })
}));

vi.mock("../../src/app/useHotkeyController", () => ({
  useHotkeyController: () => ({
    state: {
      status: "ready",
      error: null,
      snapshot: {
        revision: 1,
        actions: [{
          actionId: "capture",
          binding: "F1",
          configuredEnabled: true,
          classification: "ordinary",
          runtimeState: "registered",
          detail: null,
          actionAvailable: true,
          forceOverrideSystem: false
        }]
      },
      editor: {
        actionId: "capture",
        actionAvailable: true,
        binding: "F1",
        classificationStatus: "ready",
        classification: {
          binding: "F1",
          normalizedBinding: "F1",
          classification: "ordinary",
          message: "可以保存",
          forceOverrideAllowed: false
        },
        forceOverrideSystem: false,
        saving: false,
        error: null
      }
    },
    openEditor: mocks.openEditor,
    closeEditor: mocks.closeEditor,
    setBinding: mocks.setBinding,
    appendBindingToken: mocks.appendBindingToken,
    setForceOverrideSystem: mocks.setForceOverrideSystem,
    save: mocks.saveEditor
  })
}));

import { HotkeysPage } from "../../src/pages/hotkeys/HotkeysPage";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

describe("HotkeysPage native capture lifecycle", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
    mocks.stopCapture.mockResolvedValue(true);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    document.body.replaceChildren();
    vi.clearAllMocks();
  });

  it.each(["Escape", "取消", "保存"])("stops native capture before %s closes or saves", async (action) => {
    await act(async () => {
      root.render(<HotkeysPage onSnapshotChanged={async () => undefined} />);
    });

    if (action === "Escape") {
      const field = document.querySelector<HTMLElement>("[role='group']")!;
      await act(async () => {
        field.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
        await Promise.resolve();
      });
      expect(mocks.closeEditor).toHaveBeenCalledOnce();
    } else {
      const button = Array.from(document.querySelectorAll("button")).find(
        (candidate) => candidate.textContent === action
      );
      expect(button).toBeInstanceOf(HTMLButtonElement);
      await act(async () => (button as HTMLButtonElement).click());
      if (action === "取消") {
        expect(mocks.closeEditor).toHaveBeenCalledOnce();
      } else {
        expect(mocks.saveEditor).toHaveBeenCalledOnce();
      }
    }

    expect(mocks.stopCapture).toHaveBeenCalled();
    expect(mocks.stopCapture.mock.invocationCallOrder[0]).toBeLessThan(
      action === "保存"
        ? mocks.saveEditor.mock.invocationCallOrder[0]
        : mocks.closeEditor.mock.invocationCallOrder[0]
    );
  });

  it.each(["取消", "保存"])("keeps the editor action blocked when stop fails for %s", async (action) => {
    mocks.stopCapture.mockResolvedValue(false);
    await act(async () => {
      root.render(<HotkeysPage onSnapshotChanged={async () => undefined} />);
    });

    const button = getButton(action);
    await act(async () => {
      button.click();
      await Promise.resolve();
    });

    expect(mocks.stopCapture).toHaveBeenCalledOnce();
    expect(mocks.closeEditor).not.toHaveBeenCalled();
    expect(mocks.saveEditor).not.toHaveBeenCalled();
    expect(button.disabled).toBe(false);
    expect(document.querySelector("[role='dialog']")).not.toBeNull();
  });

  it.each(["取消", "保存"])("allows %s to retry and proceeds only after stop succeeds", async (action) => {
    mocks.stopCapture
      .mockResolvedValueOnce(false)
      .mockResolvedValueOnce(true);
    await act(async () => {
      root.render(<HotkeysPage onSnapshotChanged={async () => undefined} />);
    });

    const button = getButton(action);
    await act(async () => {
      button.click();
      await Promise.resolve();
    });
    expect(mocks.closeEditor).not.toHaveBeenCalled();
    expect(mocks.saveEditor).not.toHaveBeenCalled();

    await act(async () => {
      button.click();
      await Promise.resolve();
    });
    expect(mocks.stopCapture).toHaveBeenCalledTimes(2);
    if (action === "取消") {
      expect(mocks.closeEditor).toHaveBeenCalledOnce();
    } else {
      expect(mocks.saveEditor).toHaveBeenCalledOnce();
    }
  });

  it("disables both editor actions while stop is pending and enables retry after failure", async () => {
    const stopRequest = deferred<boolean>();
    mocks.stopCapture.mockReturnValue(stopRequest.promise);
    await act(async () => {
      root.render(<HotkeysPage onSnapshotChanged={async () => undefined} />);
    });

    const cancel = getButton("取消");
    const save = getButton("保存");
    act(() => cancel.click());
    expect(cancel.disabled).toBe(true);
    expect(save.disabled).toBe(true);

    await act(async () => stopRequest.resolve(false));
    expect(cancel.disabled).toBe(false);
    expect(save.disabled).toBe(false);
    expect(mocks.closeEditor).not.toHaveBeenCalled();
  });
});

function getButton(name: string) {
  const button = Array.from(document.querySelectorAll("button")).find(
    (candidate) => candidate.textContent === name
  );
  if (!(button instanceof HTMLButtonElement)) {
    throw new Error(`Button not found: ${name}`);
  }
  return button;
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}
