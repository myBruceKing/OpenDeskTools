// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { HotkeyControllerState } from "../../src/app/hotkeyModel";

const mocks = vi.hoisted(() => ({
  stopCapture: vi.fn(async () => true),
  startCapture: vi.fn(),
  closeEditor: vi.fn(),
  saveEditor: vi.fn(async () => undefined),
  setBinding: vi.fn(),
  appendBindingToken: vi.fn(),
  setForceOverrideSystem: vi.fn(),
  setEnabled: vi.fn(),
  openEditor: vi.fn(),
  dismissSystemHotkeyNotice: vi.fn(),
  controllerState: null as HotkeyControllerState | null
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
    state: mocks.controllerState!,
    openEditor: mocks.openEditor,
    closeEditor: mocks.closeEditor,
    setBinding: mocks.setBinding,
    appendBindingToken: mocks.appendBindingToken,
    setForceOverrideSystem: mocks.setForceOverrideSystem,
    setEnabled: mocks.setEnabled,
    save: mocks.saveEditor,
    dismissSystemHotkeyNotice: mocks.dismissSystemHotkeyNotice
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
    mocks.controllerState = readyEditorState();
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

  it("routes the per-action switch to the confirmed enable controller path", async () => {
    mocks.controllerState = {
      ...readyEditorState(),
      editor: null
    };
    await act(async () => {
      root.render(<HotkeysPage onSnapshotChanged={async () => undefined} />);
    });

    const toggle = document.querySelector<HTMLButtonElement>("[role='switch']");
    expect(toggle).toBeInstanceOf(HTMLButtonElement);
    expect(toggle?.getAttribute("aria-checked")).toBe("true");

    await act(async () => toggle?.click());

    expect(mocks.setEnabled).toHaveBeenCalledWith("capture", false);
  });

  it("blocks all enable and edit actions while an enable update is pending", async () => {
    mocks.controllerState = {
      ...readyEditorState(),
      editor: null,
      pendingEnabledActionId: "capture"
    };
    await act(async () => {
      root.render(<HotkeysPage onSnapshotChanged={async () => undefined} />);
    });

    expect(document.querySelector<HTMLButtonElement>("[role='switch']")?.disabled).toBe(true);
    expect(getButton("编辑").disabled).toBe(true);
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

  it("explains the Win+V replacement and requires confirmed registered state", async () => {
    mocks.controllerState = readyEditorState({
      actionId: "clipboardPanel",
      binding: "Win+V",
      classification: "system_reserved",
      forceOverrideSystem: true
    });
    await act(async () => {
      root.render(<HotkeysPage onSnapshotChanged={async () => undefined} />);
    });

    const dialog = document.querySelector<HTMLElement>("[role='dialog']")!;
    expect(dialog.textContent).toContain("第一次录入会替换当前绑定");
    expect(dialog.textContent).toContain("未持久化或未生效时会保留弹窗并显示原因");
    expect(dialog.textContent).toContain("只有保存后状态显示“已注册”才算生效");
    expect(dialog.textContent).toContain("此组合由 Windows 使用，需要明确确认强制覆盖");
    expect(dialog.textContent).toContain("DisabledHotkeys");
    expect(dialog.textContent).toContain("需重启资源管理器或重启电脑");
    expect(document.querySelector("[role='switch']")?.getAttribute("aria-checked")).toBe("true");
    expect(getButton("保存").disabled).toBe(false);
  });

  it("omits the DisabledHotkeys notice for system combos that are not bare Win+letter", async () => {
    mocks.controllerState = readyEditorState({
      actionId: "clipboardPanel",
      binding: "Win+Shift+S",
      classification: "system_reserved",
      forceOverrideSystem: true
    });
    await act(async () => {
      root.render(<HotkeysPage onSnapshotChanged={async () => undefined} />);
    });

    const dialog = document.querySelector<HTMLElement>("[role='dialog']")!;
    expect(dialog.textContent).toContain("此组合由 Windows 使用，需要明确确认强制覆盖");
    expect(dialog.textContent).not.toContain("DisabledHotkeys");
  });

  it("shows the Explorer restart popup when a system hotkey notice is present", async () => {
    const base = readyEditorState();
    mocks.controllerState = {
      ...base,
      editor: null,
      systemHotkeyNotice: { binding: "Win+V", letter: "V", restartRequired: true }
    };
    await act(async () => {
      root.render(<HotkeysPage onSnapshotChanged={async () => undefined} />);
    });

    const dialog = document.querySelector<HTMLElement>("[role='dialog']")!;
    expect(dialog.textContent).toContain("需要重启资源管理器");
    expect(dialog.textContent).toContain("已在系统层禁用 Win+V");

    await act(async () => getButton("知道了").click());
    expect(mocks.dismissSystemHotkeyNotice).toHaveBeenCalledOnce();
  });
});

function readyEditorState({
  actionId = "capture",
  binding = "F1",
  classification = "ordinary",
  forceOverrideSystem = false
}: {
  actionId?: "capture" | "clipboardPanel";
  binding?: string;
  classification?: "ordinary" | "system_reserved";
  forceOverrideSystem?: boolean;
} = {}): HotkeyControllerState {
  return {
    status: "ready",
    error: null,
    snapshot: {
      revision: 1,
      actions: [{
        actionId,
        binding,
        configuredEnabled: true,
        classification,
        runtimeState: "registered",
        runtimeBackend: "standard",
        detail: null,
        actionAvailable: true,
        forceOverrideSystem
      }]
    },
    editor: {
      actionId,
      actionAvailable: true,
      binding,
      inputDirty: false,
      classificationStatus: "ready",
      classification: {
        binding,
        normalizedBinding: binding,
        classification,
        message: classification === "system_reserved"
          ? "此组合由 Windows 使用，需要明确确认强制覆盖"
          : "可以保存",
        forceOverrideAllowed: classification === "system_reserved"
      },
      forceOverrideSystem,
      saving: false,
      error: null
    },
    pendingEnabledActionId: null,
    systemHotkeyNotice: null
  };
}

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
