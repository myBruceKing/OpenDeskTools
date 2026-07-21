// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { readFileSync } from "node:fs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  createClipboardReadyViewModel,
  type ClipboardControllerState,
  type ClipboardHistoryItem
} from "../../src/app/clipboardModel";
import { ClipboardSurface } from "../../src/surfaces/clipboard/ClipboardSurface";

const surfaceCss = readFileSync("src/surfaces/clipboard/ClipboardSurface.module.css", "utf8");
const historyControlsCss = readFileSync("src/components/patterns/ClipboardHistoryControls.module.css", "utf8");
const historyItemCss = readFileSync("src/components/patterns/ClipboardHistoryItem.module.css", "utf8");
const pageCss = readFileSync("src/pages/clipboard/ClipboardPage.module.css", "utf8");
const globalCss = readFileSync("src/styles/global.css", "utf8");
const surfaceSource = readFileSync("src/surfaces/clipboard/ClipboardSurface.tsx", "utf8");
const pageSource = readFileSync("src/pages/clipboard/ClipboardPage.tsx", "utf8");

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ startDragging: vi.fn(async () => undefined) })
}));

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const items: ClipboardHistoryItem[] = [
  {
    id: "1",
    revision: 2,
    kind: "text",
    textContent: "第一条剪贴板内容",
    sourceApplication: "记事本",
    sourceProcess: "notepad.exe",
    capturedAtMs: 1_720_000_000_000,
    byteSize: 24,
    isFavorite: false,
    sourceIconAvailable: false,
    fileCount: null,
    fileNames: null,
    displayCategory: "text"
  },
  {
    id: "2",
    revision: 1,
    kind: "image",
    textContent: null,
    sourceApplication: "Snipaste",
    sourceProcess: "Snipaste.exe",
    capturedAtMs: 1_720_000_001_000,
    byteSize: 1024,
    isFavorite: true,
    sourceIconAvailable: false,
    fileCount: null,
    fileNames: null,
    displayCategory: "image"
  }
];

const fileItems: ClipboardHistoryItem[] = [
  {
    id: "21", revision: 1, kind: "files", textContent: null,
    sourceApplication: "文件资源管理器", sourceProcess: "explorer.exe",
    capturedAtMs: 1_720_000_002_000, byteSize: 128, isFavorite: false,
    sourceIconAvailable: false, fileCount: 1, fileNames: ["说明.txt"], displayCategory: "text"
  },
  {
    id: "22", revision: 1, kind: "files", textContent: null,
    sourceApplication: "文件资源管理器", sourceProcess: "explorer.exe",
    capturedAtMs: 1_720_000_003_000, byteSize: 256, isFavorite: false,
    sourceIconAvailable: false, fileCount: 1, fileNames: ["截图.png"], displayCategory: "image"
  },
  {
    id: "23", revision: 1, kind: "files", textContent: null,
    sourceApplication: "文件资源管理器", sourceProcess: "explorer.exe",
    capturedAtMs: 1_720_000_004_000, byteSize: 512, isFavorite: true,
    sourceIconAvailable: false, fileCount: 2, fileNames: ["说明.txt", "截图.png"], displayCategory: "files"
  }
];

function readyState(history = items): ClipboardControllerState {
  return {
    status: "ready",
    viewModel: createClipboardReadyViewModel({
      items: history,
      totalCount: history.length,
      monitoring: "running",
      surfaceActive: true,
      inputAvailable: true
    }),
    error: null,
    realtimeError: null,
    pendingItemIds: [],
    itemAction: null,
    textEdit: null,
    surfaceActive: true,
    surfaceClosing: false,
    surfaceError: null,
    clearing: false
  };
}

describe("ClipboardSurface", () => {
  let host: HTMLDivElement;
  let root: Root;
  const onCopy = vi.fn();
  const onInput = vi.fn();
  const onClose = vi.fn(async () => true);
  const onSetFavorite = vi.fn();
  const onDelete = vi.fn();
  const loadSourceIcon = vi.fn(async () => new Blob([new Uint8Array([2])], { type: "image/png" }));
  const onOpenPreview = vi.fn(async () => undefined);
  const onClosePreview = vi.fn(async () => undefined);
  const onTracePreviewDebug = vi.fn(async () => undefined);
  let previewHoverListener: ((change: { inside: boolean; recordId: string | null }) => void) | undefined;
  const onSubscribePreviewHover = vi.fn(async (listener: typeof previewHoverListener) => {
    previewHoverListener = listener;
    return () => undefined;
  });

  beforeEach(() => {
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
    Object.defineProperty(window, "innerWidth", { configurable: true, value: 380 });
    Object.defineProperty(window, "innerHeight", { configurable: true, value: 520 });
    Object.defineProperty(URL, "createObjectURL", { configurable: true, value: vi.fn(() => "blob:surface") });
    Object.defineProperty(URL, "revokeObjectURL", { configurable: true, value: vi.fn() });
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", { configurable: true, value: vi.fn() });
    Object.defineProperty(HTMLElement.prototype, "getBoundingClientRect", {
      configurable: true,
      value() {
        return { x: 8, y: 80, left: 8, top: 80, right: 370, bottom: 150, width: 362, height: 70, toJSON() { return {}; } };
      }
    });
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    document.body.replaceChildren();
    vi.clearAllMocks();
    previewHoverListener = undefined;
  });

  async function render(state = readyState()) {
    await act(async () => root.render(
      <ClipboardSurface
        state={state}
        loadSourceIcon={loadSourceIcon}
        onCopy={onCopy}
        onInput={onInput}
        onClose={onClose}
        onSetFavorite={onSetFavorite}
        onDelete={onDelete}
        onOpenPreview={onOpenPreview}
        onClosePreview={onClosePreview}
        onSubscribePreviewHover={onSubscribePreviewHover}
        onTracePreviewDebug={onTracePreviewDebug}
      />
    ));
  }

  it("matches the selected list-only surface structure at the natural viewport", async () => {
    await render();
    expect(document.body.textContent).not.toContain("剪贴板历史（2）");
    expect(document.querySelectorAll("[role='option']")).toHaveLength(2);
    expect(document.querySelector("input[type='search']")).toBeNull();
    const tabs = document.querySelector("[role='radiogroup'][aria-label='剪贴板筛选']")!;
    const filterControl = tabs.parentElement as HTMLElement;
    expect(filterControl.getAttribute("data-clipboard-history-filter")).toBe("true");
    expect(Array.from(tabs.querySelectorAll("[role='radio']")).map((tab) => tab.textContent)).toEqual(["全部", "文本", "图片", "收藏"]);
    expect(document.querySelector("button[aria-label='关闭剪贴板面板']")).toBeTruthy();
    expect(document.querySelectorAll("[data-window-border-layer='true']")).toHaveLength(1);
    expect(document.querySelector("[data-window-border-layer='true']"))
      .toBe(document.querySelector("section[aria-label='剪贴板快捷面板']"));
    expect(document.body.textContent).not.toContain("Enter 输入");
    expect(document.querySelector("[role='option'][aria-selected='true']")?.textContent).toContain("第一条剪贴板内容");
    for (const row of document.querySelectorAll<HTMLElement>("[role='option']")) {
      const actions = row.querySelector<HTMLElement>("[data-clipboard-history-actions='true']")!;
      expect(actions).toBeTruthy();
      expect(actions.querySelectorAll("button")).toHaveLength(2);
    }
    expect(document.querySelector("button[aria-label^='更多操作：']")).toBeNull();
    expect(document.querySelectorAll("[data-clipboard-history-row-content='true']")).toHaveLength(2);
    expect(surfaceCss).not.toMatch(/\.rowActions\s*\{|rowMenu|rowDelete|--segmented-height|--segmented-item-padding-x|--segmented-font-size/);
    expect(surfaceCss).not.toMatch(/\.sourceIcon\s*\{|\.rowCopy\s*\{|\.rowMeta\s*\{|\.contextMenu\s*\{/);
    expect(surfaceCss).not.toMatch(/previewPopover|z-index:\s*var\(--z-tooltip\)/);
    expect(surfaceSource).not.toMatch(/function PreviewPopover|<PreviewPopover/);
    expect(pageCss).not.toMatch(/\.itemIcon\s*\{|\.rowCopy\s*\{|\.rowMeta\s*\{|\.rowActions\s*\{/);
    expect(historyItemCss).toMatch(/\.sourceIcon\s*\{|\.rowCopy[\s\S]*\.rowMeta[\s\S]*\.contextMenu\s*\{/);
    expect(surfaceSource).toContain("<ClipboardHistoryRowContent");
    expect(pageSource).toContain("<ClipboardHistoryRowContent");
    expect(surfaceSource).toContain("<ClipboardHistoryContextMenu");
    expect(historyControlsCss).toMatch(/\.rowActions\s*\{[\s\S]*grid-template-rows:\s*26px 26px;[\s\S]*gap:\s*2px/);
    expect(historyControlsCss).not.toMatch(/visibility:\s*hidden|opacity:\s*0;|\.row:hover|\.row:focus-within/);
  });

  it("filters the single list in place across text, image, and favorite tabs", async () => {
    await render();
    const tab = (label: string) => Array.from(document.querySelectorAll<HTMLButtonElement>("[role='radio']"))
      .find((button) => button.textContent === label)!;

    await act(async () => {
      tab("全部").focus();
      tab("全部").dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    });
    expect(tab("文本").getAttribute("aria-checked")).toBe("true");
    expect(document.activeElement).toBe(tab("文本"));
    expect(document.querySelectorAll("[role='option']")).toHaveLength(1);

    await act(async () => tab("图片").click());
    expect(document.querySelectorAll("[role='option']")).toHaveLength(1);
    expect(document.querySelector("[role='option']")?.textContent).toContain("图片内容");

    await act(async () => tab("收藏").click());
    expect(document.querySelectorAll("[role='option']")).toHaveLength(1);
    expect(document.querySelector("[role='option']")?.textContent).toContain("图片内容");

    await act(async () => tab("文本").click());
    expect(document.querySelectorAll("[role='option']")).toHaveLength(1);
    expect(document.querySelector("[role='option']")?.textContent).toContain("第一条剪贴板内容");
  });

  it("keeps the shared favorite and delete buttons visible and confirms direct deletion", async () => {
    await render();
    const firstRow = document.querySelector<HTMLElement>("[role='option']")!;
    const actions = firstRow.querySelector<HTMLElement>("[data-clipboard-history-actions='true']")!;
    await act(async () => firstRow.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));
    expect(actions.querySelectorAll("button")).toHaveLength(2);
    expect(actions.querySelector("button[aria-label='收藏 第一条剪贴板内容']")).toBeTruthy();
    expect(actions.querySelector("button[aria-label='删除 第一条剪贴板内容']")).toBeTruthy();

    await act(async () => actions.querySelector<HTMLButtonElement>("button[aria-label^='收藏 ']")!.click());
    expect(onSetFavorite).toHaveBeenCalledWith("1", true);
    await act(async () => actions.querySelector<HTMLButtonElement>("button[aria-label^='删除 ']")!.click());
    expect(document.querySelector("[role='dialog']")?.textContent).toContain("确认永久删除");
    const confirm = Array.from(document.querySelectorAll<HTMLButtonElement>("button"))
      .find((button) => button.textContent?.trim() === "删除")!;
    await act(async () => confirm.click());
    expect(onDelete).toHaveBeenCalledWith("1");
  });

  it("keeps file payload semantics while filtering by its display category", async () => {
    await render(readyState(fileItems));
    const tab = (label: string) => Array.from(document.querySelectorAll<HTMLButtonElement>("[role='radio']"))
      .find((button) => button.textContent === label)!;

    expect(document.querySelectorAll("[role='option']")).toHaveLength(3);
    await act(async () => tab("文本").click());
    expect(document.querySelectorAll("[role='option']")).toHaveLength(1);
    expect(document.querySelector("[role='option']")?.textContent).toContain("说明.txt");
    expect(document.querySelector("[role='option']")?.textContent).toContain("文件");

    await act(async () => tab("图片").click());
    expect(document.querySelectorAll("[role='option']")).toHaveLength(1);
    const imageFileRow = document.querySelector<HTMLElement>("[role='option']")!;
    expect(imageFileRow.textContent).toContain("截图.png");
    await act(async () => imageFileRow.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));
    expect(onOpenPreview).toHaveBeenLastCalledWith("22");
    expect(document.querySelector("[role='region'][aria-label^='预览：']")).toBeNull();

    await act(async () => tab("收藏").click());
    const multiFileRow = document.querySelector<HTMLElement>("[role='option']")!;
    expect(multiFileRow.textContent).toContain("2 个文件");
    await act(async () => multiFileRow.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));
    expect(onOpenPreview).toHaveBeenLastCalledWith("23");
  });

  it("keeps long rows readable at the compact viewport without a horizontal layout track", async () => {
    Object.defineProperty(window, "innerWidth", { configurable: true, value: 320 });
    Object.defineProperty(window, "innerHeight", { configurable: true, value: 420 });
    await render(readyState([{ ...items[0], textContent: "超长标题".repeat(180), sourceApplication: "超长来源应用".repeat(30) }]));
    expect(document.querySelector("[role='option'] [data-clipboard-history-row-content='true']")?.textContent).toContain("超长标题");
    const css = surfaceCss;
    expect(css).toMatch(/\.surface\s*\{[\s\S]*grid-template-rows:\s*48px minmax\(0, 1fr\) auto;/);
    const surfaceRule = css.match(/\.surface\s*\{[^}]*\}/)?.[0] ?? "";
    const sharedSurfaceRule = globalCss.match(/\[data-window-border-layer="true"\]\s*\{[^}]*\}/)?.[0] ?? "";
    expect(surfaceRule).toMatch(/position:\s*absolute/);
    expect(surfaceRule).toMatch(/inset:\s*0/);
    expect(surfaceRule).not.toMatch(/(?:width|height):\s*100%/);
    expect(surfaceRule).not.toMatch(/overflow:|border:|border-radius:|clip-path|box-shadow|#000|black/);
    expect(sharedSurfaceRule).toMatch(/box-sizing:\s*border-box/);
    expect(sharedSurfaceRule).toMatch(/overflow:\s*hidden/);
    expect(sharedSurfaceRule).toMatch(/border:\s*var\(--border-width\) solid var\(--border-default\)/);
    expect(sharedSurfaceRule).toMatch(/border-radius:\s*var\(--radius-window\)/);
    expect(historyItemCss).toMatch(/\.rowContent\s*\{[\s\S]*minmax\(0, 1fr\)/);
    expect(css).toMatch(/\.row\s*\{[\s\S]*user-select:\s*none/);
    expect(css).toMatch(/\.history\s*\{[\s\S]*overflow-y:\s*auto/);
    const rootCss = readFileSync("src/app/ClipboardSurfaceRoot.module.css", "utf8");
    const rootRule = rootCss.match(/\.windowRoot\s*\{[^}]*\}/)?.[0] ?? "";
    expect(rootRule).toMatch(/position:\s*relative/);
    expect(rootRule).toMatch(/box-sizing:\s*border-box/);
    expect(rootCss).toMatch(/\.windowRoot\s*\{[\s\S]*width:\s*100vw;[\s\S]*height:\s*100vh;[\s\S]*background:\s*var\(--border-default\);/);
    expect(rootCss).not.toMatch(/padding:\s*1px|clip-path|\.windowRoot\s*>\s*section/);
    expect(rootRule).not.toMatch(/border:|clip-path/);
    expect(globalCss).toMatch(/html\[data-window-surface="clipboard"\][\s\S]*#root\s*\{\s*background:\s*var\(--border-default\);/);
  });

  it("prevents double-click selection and invokes input exactly once", async () => {
    await render();
    const firstRow = document.querySelector<HTMLElement>("[role='option']")!;
    const secondPressAccepted = firstRow.dispatchEvent(new MouseEvent("mousedown", {
      bubbles: true,
      cancelable: true,
      detail: 2
    }));
    expect(secondPressAccepted).toBe(false);
    await act(async () => firstRow.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, cancelable: true })));
    expect(onInput).toHaveBeenCalledTimes(1);
    expect(onInput).toHaveBeenCalledWith("1");
  });

  it("renders empty and unavailable states without fake rows", async () => {
    await render(readyState([]));
    expect(document.body.textContent).toContain("暂无剪贴板历史");
    expect(document.querySelector("[role='option']")).toBeNull();

    const unavailable = readyState([]);
    await render({
      ...unavailable,
      status: "unavailable",
      error: { code: "clipboard_history_unavailable", message: "剪贴板历史服务暂时不可用，请稍后重试。", retryable: true }
    });
    expect(document.querySelector("[role='alert']")?.textContent).toContain("暂时不可用");
  });

  it("opens the independent image preview on hover without covering the popup and closes it with Escape", async () => {
    await render();
    const imageRow = document.querySelectorAll<HTMLElement>("[role='option']")[1];
    await act(async () => imageRow.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));
    expect(onOpenPreview).toHaveBeenCalledWith("2");
    expect(document.querySelector("[role='region'][aria-label^='预览：']")).toBeNull();
    await act(async () => {
      await Promise.resolve();
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
    });
    expect(onClosePreview).toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
  });

  it("keeps the independent preview open when creating its no-activate window blurs the WebView", async () => {
    await render();
    const imageRow = document.querySelectorAll<HTMLElement>("[role='option']")[1];
    await act(async () => imageRow.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));
    expect(onOpenPreview).toHaveBeenCalledWith("2");

    await act(async () => window.dispatchEvent(new Event("blur")));
    expect(onClosePreview).not.toHaveBeenCalled();
    expect(onTracePreviewDebug).toHaveBeenCalledWith("window_blur_ignored", "2");
    expect(surfaceSource).not.toContain('window.addEventListener("blur", closeOnViewportChange)');
  });

  it("serializes close behind a pending open so the late open cannot win", async () => {
    vi.useFakeTimers();
    let resolveOpen: (() => void) | undefined;
    onOpenPreview.mockImplementationOnce(() => new Promise<undefined>((resolve) => {
      resolveOpen = () => resolve(undefined);
    }));
    try {
      await render();
      const firstRow = document.querySelector<HTMLElement>("[role='option']")!;
      await act(async () => firstRow.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));
      expect(onOpenPreview).toHaveBeenCalledWith("1");

      await act(async () => firstRow.dispatchEvent(new MouseEvent("mouseout", { bubbles: true })));
      await act(async () => vi.advanceTimersByTime(241));
      expect(onClosePreview).not.toHaveBeenCalled();

      await act(async () => {
        resolveOpen?.();
        await Promise.resolve();
        await Promise.resolve();
      });
      expect(onClosePreview).toHaveBeenCalledOnce();
    } finally {
      vi.useRealTimers();
    }
  });

  it("shows independent preview command failures instead of swallowing them", async () => {
    onOpenPreview.mockRejectedValueOnce(new Error("native preview open failed"));
    await render();
    const firstRow = document.querySelector<HTMLElement>("[role='option']")!;
    await act(async () => {
      firstRow.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(document.querySelector("[role='alert']")?.textContent)
      .toContain("打开预览窗口失败：native preview open failed");

    onOpenPreview.mockResolvedValueOnce(undefined);
    onClosePreview.mockRejectedValueOnce(new Error("native preview close failed"));
    await act(async () => {
      firstRow.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
      await Promise.resolve();
      await Promise.resolve();
    });
    await act(async () => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(document.querySelector("[role='alert']")?.textContent)
      .toContain("关闭预览窗口失败：native preview close failed");
  });

  it("supports Arrow keys, Enter input, Ctrl+C copy, root Escape, and accessible delete", async () => {
    await render();
    let firstRow = document.querySelectorAll<HTMLElement>("[role='option']")[0];
    await act(async () => firstRow.focus());
    await act(async () => firstRow.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true })));
    await act(async () => new Promise<void>((resolve) => window.requestAnimationFrame(() => resolve())));
    const secondRow = document.querySelectorAll<HTMLElement>("[role='option']")[1];
    expect(secondRow.getAttribute("aria-selected")).toBe("true");
    expect(document.activeElement).toBe(secondRow);

    await act(async () => secondRow.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
    expect(onInput).toHaveBeenCalledWith("2");
    await act(async () => secondRow.dispatchEvent(new KeyboardEvent("keydown", { key: "c", ctrlKey: true, bubbles: true })));
    expect(onCopy).toHaveBeenCalledWith("2");

    await act(async () => secondRow.dispatchEvent(new KeyboardEvent("keydown", { key: "Delete", bubbles: true })));
    expect(document.querySelector("[role='dialog']")).toBeTruthy();
    const confirm = Array.from(document.querySelectorAll<HTMLButtonElement>("button"))
      .find((button) => button.textContent?.trim() === "删除")!;
    await act(async () => confirm.click());
    expect(onDelete).toHaveBeenCalledWith("2");

    expect(document.querySelector("[role='region'][aria-label^='预览：']")).toBeNull();
    const closeButton = document.querySelector<HTMLButtonElement>("button[aria-label='关闭剪贴板面板']")!;
    await act(async () => {
      closeButton.focus();
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
    });
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("opens the same accessible actions from right-click and Shift+F10", async () => {
    await render();
    const firstRow = document.querySelectorAll<HTMLElement>("[role='option']")[0];
    await act(async () => firstRow.dispatchEvent(new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
      clientX: 360,
      clientY: 500
    })));
    let menu = document.querySelector<HTMLElement>("[role='menu']")!;
    expect(menu).toBeTruthy();
    expect(menu.getBoundingClientRect().right).toBeLessThanOrEqual(380);
    const favorite = Array.from(menu.querySelectorAll<HTMLButtonElement>("[role='menuitem']"))
      .find((button) => button.textContent?.trim() === "收藏")!;
    await act(async () => favorite.click());
    expect(onSetFavorite).toHaveBeenCalledWith("1", true);

    await act(async () => {
      firstRow.focus();
      firstRow.dispatchEvent(new KeyboardEvent("keydown", { key: "F10", shiftKey: true, bubbles: true }));
    });
    menu = document.querySelector<HTMLElement>("[role='menu']")!;
    expect(menu).toBeTruthy();
    await act(async () => menu.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
    await act(async () => new Promise<void>((resolve) => window.requestAnimationFrame(() => resolve())));
    expect(document.querySelector("[role='menu']")).toBeNull();
    expect(document.activeElement).toBe(firstRow);
  });

  it("closes the transient menu on outside press, row switch, filter, scroll, blur, and close", async () => {
    await render();
    const rows = () => document.querySelectorAll<HTMLElement>("[role='option']");
    const openMouseMenu = async () => {
      await act(async () => rows()[0].dispatchEvent(new MouseEvent("contextmenu", {
        bubbles: true,
        cancelable: true,
        clientX: 80,
        clientY: 120
      })));
      expect(document.querySelector("[role='menu']")).toBeTruthy();
    };

    const closeButton = document.querySelector<HTMLButtonElement>("button[aria-label='关闭剪贴板面板']")!;
    closeButton.focus();
    await openMouseMenu();
    expect(document.activeElement).toBe(closeButton);
    await act(async () => document.body.click());
    expect(document.querySelector("[role='menu']")).toBeNull();

    await openMouseMenu();
    await act(async () => rows()[1].click());
    expect(document.querySelector("[role='menu']")).toBeNull();

    await openMouseMenu();
    const imageTab = Array.from(document.querySelectorAll<HTMLButtonElement>("[role='radio']"))
      .find((button) => button.textContent === "图片")!;
    await act(async () => imageTab.click());
    expect(document.querySelector("[role='menu']")).toBeNull();

    const allTab = Array.from(document.querySelectorAll<HTMLButtonElement>("[role='radio']"))
      .find((button) => button.textContent === "全部")!;
    await act(async () => allTab.click());
    await openMouseMenu();
    await act(async () => document.querySelector<HTMLElement>("[role='listbox']")!
      .dispatchEvent(new Event("scroll", { bubbles: true })));
    expect(document.querySelector("[role='menu']")).toBeNull();

    await openMouseMenu();
    await act(async () => window.dispatchEvent(new Event("blur")));
    expect(document.querySelector("[role='menu']")).toBeNull();

    await openMouseMenu();
    await act(async () => closeButton.click());
    expect(document.querySelector("[role='menu']")).toBeNull();
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("uses roving keyboard focus inside the menu and restores the invoking row", async () => {
    await render();
    const firstRow = document.querySelector<HTMLElement>("[role='option']")!;
    await act(async () => {
      firstRow.focus();
      firstRow.dispatchEvent(new KeyboardEvent("keydown", { key: "F10", shiftKey: true, bubbles: true }));
    });
    const menu = document.querySelector<HTMLElement>("[role='menu']")!;
    const menuItems = menu.querySelectorAll<HTMLButtonElement>("[role='menuitem']");
    expect(document.activeElement).toBe(menuItems[0]);
    await act(async () => menu.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true })));
    expect(document.activeElement).toBe(menuItems[1]);
    await act(async () => menu.dispatchEvent(new KeyboardEvent("keydown", { key: "End", bubbles: true })));
    expect(document.activeElement).toBe(menuItems[2]);
    await act(async () => menu.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    await act(async () => new Promise<void>((resolve) => window.requestAnimationFrame(() => resolve())));
    expect(document.querySelector("[role='menu']")).toBeNull();
    expect(document.activeElement).toBe(firstRow);
  });

  it("keeps the independent preview open across the row-to-window pointer buffer", async () => {
    vi.useFakeTimers();
    try {
      await render();
      const firstRow = document.querySelector<HTMLElement>("[role='option']")!;
      await act(async () => firstRow.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));
      expect(onOpenPreview).toHaveBeenCalledWith("1");

      await act(async () => firstRow.dispatchEvent(new MouseEvent("mouseout", { bubbles: true })));
      await act(async () => vi.advanceTimersByTime(160));
      expect(onClosePreview).not.toHaveBeenCalled();

      await act(async () => previewHoverListener?.({ inside: true, recordId: "1" }));
      await act(async () => vi.advanceTimersByTime(300));
      expect(onClosePreview).not.toHaveBeenCalled();

      await act(async () => previewHoverListener?.({ inside: false, recordId: "1" }));
      await act(async () => vi.advanceTimersByTime(241));
      expect(onClosePreview).toHaveBeenCalledOnce();
    } finally {
      vi.useRealTimers();
    }
  });

  it("removes menu residue before deletion and resets local state across hide and reopen", async () => {
    await render();
    const imageTab = Array.from(document.querySelectorAll<HTMLButtonElement>("[role='radio']"))
      .find((button) => button.textContent === "图片")!;
    await act(async () => imageTab.click());
    const imageRow = document.querySelector<HTMLElement>("[role='option']")!;
    await act(async () => imageRow.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));
    expect(onOpenPreview).toHaveBeenCalledWith("2");
    await act(async () => imageRow.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true })));
    expect(onClosePreview).toHaveBeenCalled();
    const deleteMenuItem = Array.from(document.querySelectorAll<HTMLButtonElement>("[role='menuitem']"))
      .find((button) => button.textContent?.trim() === "删除")!;
    await act(async () => deleteMenuItem.click());
    expect(document.querySelector("[role='menu']")).toBeNull();
    expect(document.querySelector("[role='dialog']")).toBeTruthy();

    await render({ ...readyState(), surfaceActive: false });
    expect(document.querySelector("[role='dialog']")).toBeNull();
    expect(document.querySelector("[role='menu']")).toBeNull();

    const reopened = readyState();
    await render({ ...reopened, surfaceActive: true });
    const allTab = Array.from(document.querySelectorAll<HTMLButtonElement>("[role='radio']"))
      .find((button) => button.textContent === "全部")!;
    expect(allTab.getAttribute("aria-checked")).toBe("true");
    expect(document.querySelector("[role='option'][aria-selected='true']")?.textContent).toContain("第一条剪贴板内容");
  });

  it("does not replay an old item action after the hidden surface is reopened", async () => {
    const action = {
      action: "input" as const,
      itemId: "1",
      status: "error" as const,
      message: "目标窗口已不可用，请重新选择目标。",
      code: "clipboard_target_unavailable",
      retryable: false
    };
    await render({ ...readyState(), itemAction: action });
    expect(document.querySelector("[role='alert']")?.textContent).toContain("目标窗口已不可用");
    await render({ ...readyState(), surfaceActive: false, itemAction: action });
    expect(document.body.textContent).not.toContain("目标窗口已不可用");
    await render({ ...readyState(), surfaceActive: true, itemAction: action });
    expect(document.body.textContent).not.toContain("目标窗口已不可用");
  });

  it("renders close failures and explains browse-only input without stealing initial focus", async () => {
    const browse = readyState();
    await render({
      ...browse,
      viewModel: { ...browse.viewModel, actions: { ...browse.viewModel.actions, canTypeIntoTarget: false } },
      surfaceError: { code: "clipboard_surface_restore_failed", message: "恢复目标窗口失败，请重试。", retryable: true }
    });
    expect(document.querySelector("[role='alert']")?.textContent).toContain("恢复目标窗口失败");
    const firstRow = document.querySelector<HTMLElement>("[role='option']")!;
    expect(firstRow.getAttribute("aria-describedby")).toBe("clipboard-surface-browse-notice");
    expect(firstRow.title).toContain("仅可浏览");
    expect(document.activeElement).toBe(document.body);
    await act(async () => firstRow.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })));
    expect(onInput).not.toHaveBeenCalled();

    await render({
      ...browse,
      viewModel: { ...browse.viewModel, actions: { ...browse.viewModel.actions, canTypeIntoTarget: false } },
      surfaceError: null
    });
    expect(document.getElementById("clipboard-surface-browse-notice")?.textContent).toContain("复制后");
    expect(document.querySelector("[role='status']")?.textContent).toContain("仅浏览");
  });

  it("shows input failures as a short overlay status", async () => {
    await render({
      ...readyState(),
      itemAction: {
        action: "input",
        itemId: "1",
        status: "error",
        message: "目标窗口已不可用，请重新选择目标。",
        code: "clipboard_target_unavailable",
        retryable: false
      }
    });
    expect(document.querySelector("[role='alert']")?.textContent).toContain("目标窗口已不可用");
  });

  it("automatically clears completed item action overlays", async () => {
    vi.useFakeTimers();
    try {
      await render({
        ...readyState(),
        itemAction: {
          action: "copy",
          itemId: "1",
          status: "success",
          message: "已复制。",
          code: null,
          retryable: false
        }
      });
      expect(document.querySelector("[role='status']")?.textContent).toContain("已复制");
      await act(async () => vi.advanceTimersByTime(2601));
      expect(document.body.textContent).not.toContain("已复制");
    } finally {
      vi.useRealTimers();
    }
  });

});
