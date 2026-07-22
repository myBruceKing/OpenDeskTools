// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { readFileSync } from "node:fs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  createClipboardLoadingState,
  createClipboardReadyViewModel,
  type ClipboardControllerState,
  type ClipboardHistoryItem
} from "../../src/app/clipboardModel";
import { ClipboardPage } from "../../src/pages/clipboard/ClipboardPage";

const primitivesCss = readFileSync(
  "src/components/primitives/primitives.module.css",
  "utf8"
);
const clipboardCss = readFileSync("src/pages/clipboard/ClipboardPage.module.css", "utf8");
const imagePreviewCss = readFileSync("src/components/patterns/ImagePreview.module.css", "utf8");
const historyControlsCss = readFileSync("src/components/patterns/ClipboardHistoryControls.module.css", "utf8");
const historyItemCss = readFileSync("src/components/patterns/ClipboardHistoryItem.module.css", "utf8");

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const rawItems: ClipboardHistoryItem[] = [
  {
    id: "1",
    revision: 1,
    kind: "text",
    textContent: "普通内容",
    sourceApplication: null,
    sourceProcess: null,
    capturedAtMs: 1_720_000_000_000,
    byteSize: 12,
    isFavorite: false,
    sourceIconAvailable: false,
    fileCount: null,
    fileNames: null,
    displayCategory: "text"
  },
  {
    id: "2",
    revision: 1,
    kind: "text",
    textContent: "收藏内容",
    sourceApplication: "记事本",
    sourceProcess: "notepad.exe",
    capturedAtMs: 1_720_000_001_000,
    byteSize: 12,
    isFavorite: true,
    sourceIconAvailable: false,
    fileCount: null,
    fileNames: null,
    displayCategory: "text"
  }
];

const imageItems: ClipboardHistoryItem[] = [
  {
    id: "10",
    revision: 1,
    kind: "image",
    textContent: null,
    sourceApplication: "截图工具",
    sourceProcess: "capture.exe",
    capturedAtMs: 1_720_000_002_000,
    byteSize: 512,
    isFavorite: false,
    sourceIconAvailable: false,
    fileCount: null,
    fileNames: null,
    displayCategory: "image"
  },
  rawItems[0],
  {
    id: "11",
    revision: 1,
    kind: "image",
    textContent: null,
    sourceApplication: "画图",
    sourceProcess: "mspaint.exe",
    capturedAtMs: 1_720_000_003_000,
    byteSize: 1024,
    isFavorite: false,
    sourceIconAvailable: false,
    fileCount: null,
    fileNames: null,
    displayCategory: "image"
  }
];

const fileItems: ClipboardHistoryItem[] = [
  {
    id: "20", revision: 1, kind: "files", textContent: null,
    sourceApplication: "文件资源管理器", sourceProcess: "explorer.exe",
    capturedAtMs: 1_720_000_004_000, byteSize: 128, isFavorite: false,
    sourceIconAvailable: false, fileCount: 1, fileNames: ["说明.txt"], displayCategory: "text"
  },
  {
    id: "21", revision: 1, kind: "files", textContent: null,
    sourceApplication: "文件资源管理器", sourceProcess: "explorer.exe",
    capturedAtMs: 1_720_000_005_000, byteSize: 256, isFavorite: false,
    sourceIconAvailable: false, fileCount: 1, fileNames: ["截图.png"], displayCategory: "image"
  },
  {
    id: "22", revision: 1, kind: "files", textContent: null,
    sourceApplication: "文件资源管理器", sourceProcess: "explorer.exe",
    capturedAtMs: 1_720_000_006_000, byteSize: 384, isFavorite: true,
    sourceIconAvailable: false, fileCount: 2, fileNames: ["说明.txt", "截图.png"], displayCategory: "files"
  }
];

const longSourceApplication = "超长应用".repeat(64);
const longSourceProcess = `C:\\${"deep-folder\\".repeat(60)}app.exe`.slice(0, 512);

function readyState(overrides: Partial<ClipboardControllerState> = {}): ClipboardControllerState {
  const state: ClipboardControllerState = {
    status: "ready",
    viewModel: createClipboardReadyViewModel({
      items: rawItems,
      totalCount: 2,
      monitoring: "running",
      surfaceActive: false,
      inputAvailable: false
    }),
    error: null,
    realtimeError: null,
    pendingItemIds: [],
    itemAction: null,
    textEdit: null,
    surfaceActive: false,
    surfaceClosing: false,
    surfaceError: null,
    clearing: false
  };
  return Object.assign(state, overrides);
}

describe("ClipboardPage", () => {
  let host: HTMLDivElement;
  let root: Root;
  const onSetFavorite = vi.fn();
  const onUpdateText = vi.fn(async () => true);
  const onDelete = vi.fn();
  const onClear = vi.fn();
  const loadImage = vi.fn(async () => new Blob([new Uint8Array([1])], { type: "image/png" }));
  const loadSourceIcon = vi.fn(async () => new Blob([new Uint8Array([1])], { type: "image/png" }));

  beforeEach(() => {
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
    let urlIndex = 0;
    Object.defineProperty(URL, "createObjectURL", {
      configurable: true,
      value: vi.fn(() => `blob:page-${++urlIndex}`)
    });
    Object.defineProperty(URL, "revokeObjectURL", {
      configurable: true,
      value: vi.fn()
    });
    loadImage.mockResolvedValue(new Blob([new Uint8Array([1])], { type: "image/png" }));
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true,
      value: vi.fn()
    });
    Object.defineProperty(window, "innerWidth", { configurable: true, value: 320 });
    Object.defineProperty(window, "innerHeight", { configurable: true, value: 480 });
    Object.defineProperty(HTMLElement.prototype, "offsetWidth", {
      configurable: true,
      get() {
        return this.dataset.tooltipContainer === "true" ? 300 : 0;
      }
    });
    Object.defineProperty(HTMLElement.prototype, "offsetHeight", {
      configurable: true,
      get() {
        return this.dataset.tooltipContainer === "true" ? 280 : 0;
      }
    });
    Object.defineProperty(HTMLElement.prototype, "clientHeight", {
      configurable: true,
      get() {
        return this.getAttribute("aria-label") === "查看内容信息详情" ? 280 : 0;
      }
    });
    Object.defineProperty(HTMLElement.prototype, "scrollHeight", {
      configurable: true,
      get() {
        return this.getAttribute("aria-label") === "查看内容信息详情" ? 600 : 0;
      }
    });
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    document.body.replaceChildren();
    vi.useRealTimers();
    vi.clearAllMocks();
  });

  async function render(state: ClipboardControllerState) {
    await act(async () => {
      root.render(
        <ClipboardPage
          state={state}
          loadImage={loadImage}
          loadSourceIcon={loadSourceIcon}
          onUpdateText={onUpdateText}
          onSetFavorite={onSetFavorite}
          onDelete={onDelete}
          onClearUnfavoriteHistory={onClear}
        />
      );
    });
  }

  it("shows honest loading and unavailable states with mutations disabled", async () => {
    await render(createClipboardLoadingState());
    expect(document.body.textContent).toContain("正在加载剪贴板历史");
    expect(document.body.textContent).toContain("监控已暂停");
    expect(document.querySelector<HTMLButtonElement>("button[aria-label='收藏']")?.disabled).not.toBe(false);

    const unavailable = createClipboardLoadingState();
    await render({
      ...unavailable,
      status: "unavailable",
      viewModel: { ...unavailable.viewModel, monitoring: "unavailable" },
      error: {
        code: "clipboard_history_unavailable",
        message: "剪贴板历史服务暂时不可用，请稍后重试。",
        retryable: true
      }
    });
    expect(document.body.textContent).toContain("剪贴板历史服务暂时不可用");
    expect(document.body.textContent).toContain("监控不可用");
    const clearButton = Array.from(document.querySelectorAll<HTMLButtonElement>("button"))
      .find((button) => button.textContent?.trim() === "清空历史");
    expect(clearButton?.disabled).toBe(true);
  });

  it("requests a real favorite mutation without changing the confirmed row locally", async () => {
    await render(readyState());
    const favoriteButton = document.querySelector<HTMLButtonElement>("button[aria-label='收藏 普通内容']")!;
    expect(favoriteButton).toBeTruthy();
    await act(async () => favoriteButton.click());

    expect(onSetFavorite).toHaveBeenCalledWith("1", true);
    expect(document.querySelector("button[aria-label='收藏 普通内容']")).toBeTruthy();
    expect(document.querySelectorAll("button[aria-label^='取消收藏 ']")).toHaveLength(1);
  });

  it("keeps favorite above delete on each history row and removes both from content preview", async () => {
    await render(readyState());
    const rows = document.querySelectorAll<HTMLElement>("[role='option']");
    const filterControl = document.querySelector<HTMLElement>("[data-clipboard-history-filter='true']")!;
    expect(filterControl.querySelector(":scope > [role='radiogroup']")).toBeTruthy();
    expect(rows[0].querySelectorAll("button")).toHaveLength(2);
    expect(rows[0].querySelector("[data-clipboard-history-actions='true']")).toBeTruthy();
    expect(rows[0].querySelectorAll("button")[0].getAttribute("aria-label")).toBe("收藏 普通内容");
    expect(rows[0].querySelectorAll("button")[1].getAttribute("aria-label")).toBe("删除 普通内容");
    const deleteButton = rows[0].querySelectorAll<HTMLButtonElement>("button")[1];
    await act(async () => {
      deleteButton.focus();
      deleteButton.click();
    });
    expect(document.querySelector("[role='dialog']")?.textContent).toContain("确认永久删除");
    await act(async () => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
    });
    expect(document.querySelector("[role='dialog']")).toBeNull();
    expect(document.activeElement).toBe(deleteButton);
    const preview = document.querySelector<HTMLElement>("[aria-label='双击编辑剪贴板文本']")?.closest("section");
    expect(preview?.querySelector("button[aria-label^='收藏'], button[aria-label^='取消收藏'], button[aria-label^='删除 ']")).toBeNull();
    expect(historyControlsCss).toMatch(/\.rowActions\s*\{[\s\S]*grid-template-rows:\s*26px 26px;[\s\S]*gap:\s*2px/);
    expect(clipboardCss).not.toMatch(/\.rowActions\s*\{/);
  });

  it("edits text by double-click and saves with Ctrl+Enter using the current revision", async () => {
    await render(readyState());
    const preview = document.querySelector<HTMLElement>("[aria-label='双击编辑剪贴板文本']")!;
    await act(async () => preview.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })));
    const editor = document.querySelector<HTMLTextAreaElement>("textarea[aria-label='编辑剪贴板文本']")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set?.call(editor, "修改后的内容");
      editor.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => {
      editor.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", ctrlKey: true, bubbles: true }));
      await Promise.resolve();
    });
    expect(onUpdateText).toHaveBeenCalledWith("1", "修改后的内容", 1);
    expect(document.querySelector("textarea[aria-label='编辑剪贴板文本']")).toBeNull();
    await act(async () => new Promise<void>((resolve) => window.requestAnimationFrame(() => resolve())));
    expect(document.activeElement).toBe(document.querySelector("[aria-label='双击编辑剪贴板文本']"));
  });

  it("saves on blur and cancels with Escape without mutating images", async () => {
    await render(readyState());
    const preview = document.querySelector<HTMLElement>("[aria-label='双击编辑剪贴板文本']")!;
    await act(async () => preview.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })));
    let editor = document.querySelector<HTMLTextAreaElement>("textarea[aria-label='编辑剪贴板文本']")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set?.call(editor, "失焦保存");
      editor.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => {
      editor.focus();
      editor.blur();
      await Promise.resolve();
    });
    expect(onUpdateText).toHaveBeenCalledWith("1", "失焦保存", 1);

    onUpdateText.mockClear();
    await render(readyState());
    const refreshedPreview = document.querySelector<HTMLElement>("[aria-label='双击编辑剪贴板文本']")!;
    await act(async () => refreshedPreview.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })));
    editor = document.querySelector<HTMLTextAreaElement>("textarea[aria-label='编辑剪贴板文本']")!;
    await act(async () => editor.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    expect(onUpdateText).not.toHaveBeenCalled();

    await render(readyState({
      viewModel: createClipboardReadyViewModel({
        items: imageItems,
        totalCount: imageItems.length,
        monitoring: "running",
        surfaceActive: false,
        inputAvailable: false
      })
    }));
    const imageRow = Array.from(document.querySelectorAll<HTMLElement>("[role='option']"))
      .find((row) => row.textContent?.includes("图片内容"))!;
    await act(async () => imageRow.click());
    expect(document.querySelector("[aria-label='双击编辑剪贴板文本']")).toBeNull();
  });

  it.each([
    ["pending", "正在保存…", null],
    ["success", "已保存。", null],
    ["error", "已存在相同内容，未保存。", "clipboard_edit_duplicate"],
    ["error", "内容不能为空。", "clipboard_edit_empty"],
    ["error", "内容已在其他位置更新，请重新编辑。", "clipboard_revision_conflict"]
  ] as const)("shows the %s edit state", async (status, message, code) => {
    await render(readyState({
      textEdit: {
        itemId: "1",
        status,
        message,
        code,
        retryable: code === "clipboard_revision_conflict"
      }
    }));
    expect(document.body.textContent).toContain(message);
    if (status === "error") expect(document.querySelector("[role='alert']")?.textContent).toContain(message);
  });

  it("filters only the loaded records and retains unavailable source text", async () => {
    await render(readyState());
    expect(document.body.textContent).toContain("来源不可用");
    const search = document.querySelector<HTMLInputElement>("input[placeholder='搜索剪贴板内容']")!;
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      setter?.call(search, "收藏内容");
      search.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(document.body.textContent).not.toContain("普通内容");
    expect(document.body.textContent).toContain("收藏内容");
  });

  it("keeps unsaved settings while a same-value history refresh rerenders the page", async () => {
    await render(readyState());
    const maxItems = Array.from(document.querySelectorAll<HTMLInputElement>("input"))
      .find((input) => input.value === "100")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(maxItems, "240");
      maxItems.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(maxItems.value).toBe("240");

    await render({
      ...readyState(),
      viewModel: createClipboardReadyViewModel({
        items: [{ ...rawItems[0] }, { ...rawItems[1] }],
        totalCount: 2,
        monitoring: "running",
        surfaceActive: false,
        inputAvailable: false
      })
    });
    expect(Array.from(document.querySelectorAll<HTMLInputElement>("input"))
      .find((input) => input.value === "240")).toBeTruthy();
  });

  it("shows file records without losing file semantics and uses display categories for tabs", async () => {
    await render(readyState({
      viewModel: createClipboardReadyViewModel({
        items: fileItems,
        totalCount: fileItems.length,
        monitoring: "running",
        surfaceActive: false,
        inputAvailable: false
      })
    }));
    const tab = (label: string) => Array.from(document.querySelectorAll<HTMLButtonElement>("[role='radio']"))
      .find((button) => button.textContent === label)!;

    expect(document.querySelectorAll("[role='option']")).toHaveLength(3);
    await act(async () => tab("文本").click());
    expect(document.querySelectorAll("[role='option']")).toHaveLength(1);
    expect(document.querySelector("[role='option']")?.textContent).toContain("说明.txt");
    expect(document.querySelector("[role='option']")?.textContent).toContain("文件");

    await act(async () => tab("图片").click());
    expect(document.querySelectorAll("[role='option']")).toHaveLength(1);
    expect(document.querySelector("[role='option']")?.textContent).toContain("截图.png");
    expect(loadImage).not.toHaveBeenCalled();

    await act(async () => tab("收藏").click());
    expect(document.querySelector("[role='option']")?.textContent).toContain("2 个文件");
    expect(document.body.textContent).toContain("说明.txt");
    expect(document.body.textContent).toContain("截图.png");
    expect(document.querySelector("[aria-label='双击编辑剪贴板文本']")).toBeNull();
  });

  it("lazy-loads only the selected image while keyboard selection keeps the list active", async () => {
    await render({
      ...readyState(),
      viewModel: createClipboardReadyViewModel({
        items: imageItems,
        totalCount: imageItems.length,
        monitoring: "running",
        surfaceActive: false,
        inputAvailable: false
      })
    });
    await act(async () => Promise.resolve());
    expect(loadImage).toHaveBeenCalledTimes(1);
    expect(loadImage).toHaveBeenLastCalledWith("10");

    const list = document.querySelector<HTMLElement>("[role='listbox']")!;
    list.focus();
    await act(async () => {
      list.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    });
    expect(document.activeElement).toBe(list);
    expect(loadImage).toHaveBeenCalledTimes(1);
    expect(list.getAttribute("aria-activedescendant")).toContain("1");

    await act(async () => {
      list.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
      await Promise.resolve();
    });
    expect(loadImage).toHaveBeenCalledTimes(2);
    expect(loadImage).toHaveBeenLastCalledWith("11");
    expect(document.activeElement).toBe(list);
  });

  it("releases the selected image before delete and clear mutations", async () => {
    const imageState = {
      ...readyState(),
      viewModel: createClipboardReadyViewModel({
        items: [imageItems[0]],
        totalCount: 1,
        monitoring: "running",
        surfaceActive: false,
        inputAvailable: false
      })
    };
    await render(imageState);
    await act(async () => Promise.resolve());

    const deleteButton = Array.from(document.querySelectorAll<HTMLButtonElement>("button"))
      .find((button) => button.getAttribute("aria-label") === "删除 图片内容")!;
    await act(async () => deleteButton.click());
    const deleteConfirm = Array.from(document.querySelectorAll<HTMLButtonElement>("button"))
      .find((button) => button.textContent?.trim() === "删除" && button !== deleteButton)!;
    await act(async () => deleteConfirm.click());
    expect(URL.revokeObjectURL).toHaveBeenCalledTimes(1);
    expect(onDelete).toHaveBeenCalledWith("10");

    const retry = Array.from(document.querySelectorAll<HTMLButtonElement>("button"))
      .find((button) => button.textContent?.trim() === "重试")!;
    await act(async () => {
      retry.click();
      await Promise.resolve();
    });
    const clearButton = Array.from(document.querySelectorAll<HTMLButtonElement>("button"))
      .find((button) => button.textContent?.trim() === "清空历史")!;
    await act(async () => clearButton.click());
    const clearConfirm = Array.from(document.querySelectorAll<HTMLButtonElement>("button"))
      .find((button) => button.textContent?.trim() === "清空")!;
    await act(async () => clearConfirm.click());
    expect(URL.revokeObjectURL).toHaveBeenCalledTimes(2);
    expect(onClear).toHaveBeenCalledOnce();
  });

  it("keeps landscape, portrait, and transparent images contained at 960 by 640", () => {
    expect(clipboardCss).toMatch(/\.page\s*\{[\s\S]*min-width:\s*0;[\s\S]*overflow:\s*hidden;/);
    expect(clipboardCss).toMatch(/\.previewBox\s*\{[\s\S]*min-height:\s*0;[\s\S]*overflow:\s*hidden;/);
    expect(clipboardCss).toMatch(/\.detailsContent\s*\{[\s\S]*grid-template-rows:\s*minmax\(0, 1fr\) auto/);
    expect(clipboardCss).toMatch(/\.detailsFooter\s*\{[\s\S]*display:\s*flex;[\s\S]*border-top:/);
    expect(clipboardCss).not.toMatch(/\.actionButtons\s*\{/);
    expect(clipboardCss).not.toMatch(/\.actionButton\s*\{/);
    expect(imagePreviewCss).toMatch(/\.image\s*\{[\s\S]*width:\s*100%;[\s\S]*height:\s*100%;[\s\S]*object-fit:\s*contain;/);
    expect(imagePreviewCss).toMatch(/\.imageStage\s*\{[\s\S]*background-image:[\s\S]*linear-gradient/);
  });

  it("confirms backend delete and clear operations with permanent-action copy", async () => {
    await render(readyState());
    const deleteButton = Array.from(document.querySelectorAll<HTMLButtonElement>("button"))
      .find((button) => button.getAttribute("aria-label") === "删除 普通内容")!;
    await act(async () => deleteButton.click());
    expect(document.body.textContent).toContain("确认永久删除");
    const deleteConfirm = Array.from(document.querySelectorAll<HTMLButtonElement>("button"))
      .find((button) => button.textContent?.trim() === "删除" && button !== deleteButton)!;
    await act(async () => deleteConfirm.click());
    expect(onDelete).toHaveBeenCalledWith("1");

    const clearButton = Array.from(document.querySelectorAll<HTMLButtonElement>("button"))
      .find((button) => button.textContent?.trim() === "清空历史")!;
    await act(async () => clearButton.click());
    expect(document.body.textContent).toContain("已收藏内容会保留");
    const clearConfirm = Array.from(document.querySelectorAll<HTMLButtonElement>("button"))
      .find((button) => button.textContent?.trim() === "清空")!;
    await act(async () => clearConfirm.click());
    expect(onClear).toHaveBeenCalledOnce();
  });

  it.each([
    ["empty", []],
    ["all-favorite", rawItems.map((item) => ({ ...item, isFavorite: true }))]
  ] as const)("disables clear when history is %s", async (_label, items) => {
    await render({
      ...readyState(),
      viewModel: createClipboardReadyViewModel({
        items: [...items],
        totalCount: items.length,
        monitoring: "running",
        surfaceActive: false,
        inputAvailable: false
      })
    });
    const clearButton = Array.from(document.querySelectorAll<HTMLButtonElement>("button"))
      .find((button) => button.textContent?.trim() === "清空历史");
    expect(clearButton?.disabled).toBe(true);
  });

  it("enables clear only when a loaded unfavorite record exists", async () => {
    await render(readyState());
    const clearButton = Array.from(document.querySelectorAll<HTMLButtonElement>("button"))
      .find((button) => button.textContent?.trim() === "清空历史");
    expect(clearButton?.disabled).toBe(false);
  });

  it("uses the shared clipboard row content and preview contracts", async () => {
    await render(readyState());
    expect(document.querySelectorAll("[data-clipboard-history-row-content='true']")).toHaveLength(2);
    expect(clipboardCss).not.toMatch(/\.itemIcon\s*\{|\.rowCopy\s*\{|\.rowMeta\s*\{|\.rowActions\s*\{/);
    expect(historyItemCss).toMatch(/\.sourceIcon\s*\{|\.rowCopy[\s\S]*\.rowMeta/);

    await render({
      ...readyState(),
      viewModel: createClipboardReadyViewModel({
        items: [imageItems[0]], totalCount: 1, monitoring: "running", surfaceActive: false, inputAvailable: false
      })
    });
    expect(document.querySelector("[data-clipboard-history-preview-content='true']")).toBeTruthy();
  });

  it("shows realtime unavailability while keeping confirmed history readable", async () => {
    await render({
      ...readyState(),
      viewModel: {
        ...readyState().viewModel,
        monitoring: "unavailable"
      },
      realtimeError: {
        code: "clipboard_subscription_unavailable",
        message: "剪贴板实时更新暂时不可用，当前历史仍可查看。",
        retryable: true
      }
    });
    expect(document.body.textContent).toContain("监控不可用");
    expect(document.body.textContent).toContain("剪贴板实时更新暂时不可用");
    expect(document.body.textContent).toContain("普通内容");
  });

  it.each([
    ["unknown", rawItems[0], "来源应用：来源不可用", "来源进程：来源不可用"],
    ["ordinary", rawItems[1], "来源应用：记事本", "来源进程：notepad.exe"]
  ] as const)("shows complete %s source metadata in the details tooltip", async (_label, item, appCopy, processCopy) => {
    await render({
      ...readyState(),
      viewModel: createClipboardReadyViewModel({
        items: [{ ...item }],
        totalCount: 1,
        monitoring: "running",
        surfaceActive: false,
        inputAvailable: false
      })
    });
    const hint = document.querySelector<HTMLElement>("[aria-label='查看内容信息']")!;
    expect(hint.getAttribute("role")).toBe("button");
    expect(hint.getAttribute("aria-expanded")).toBe("false");
    const ordinaryHint = document.querySelector<HTMLElement>("[aria-label='查看剪贴板历史提示']")!;
    expect(ordinaryHint.getAttribute("role")).toBe("img");
    expect(ordinaryHint.hasAttribute("aria-expanded")).toBe(false);
    Object.defineProperty(hint, "getBoundingClientRect", {
      configurable: true,
      value: () => ({
        left: 292,
        right: 310,
        top: 420,
        bottom: 438,
        width: 18,
        height: 18,
        x: 292,
        y: 420,
        toJSON: () => undefined
      })
    });
    await act(async () => hint.focus());
    const tooltip = document.querySelector<HTMLElement>("[role='dialog'][aria-label='查看内容信息详情']")!;
    expect(tooltip.textContent).toContain(appCopy);
    expect(tooltip.textContent).toContain(processCopy);
    expect(hint.getAttribute("aria-controls")).toBe(tooltip.id);
    expect(hint.getAttribute("aria-expanded")).toBe("true");
  });

  it("wraps and clamps maximum source metadata when opened from the keyboard", async () => {
    const longItem: ClipboardHistoryItem = {
      ...rawItems[0],
      id: "3",
      sourceApplication: longSourceApplication,
      sourceProcess: longSourceProcess
    };
    await render({
      ...readyState(),
      viewModel: createClipboardReadyViewModel({
        items: [longItem],
        totalCount: 1,
        monitoring: "running",
        surfaceActive: false,
        inputAvailable: false
      })
    });
    const hint = document.querySelector<HTMLElement>("[aria-label='查看内容信息']")!;
    Object.defineProperty(hint, "getBoundingClientRect", {
      configurable: true,
      value: () => ({
        left: 2,
        right: 20,
        top: 10,
        bottom: 28,
        width: 18,
        height: 18,
        x: 2,
        y: 10,
        toJSON: () => undefined
      })
    });

    await act(async () => hint.focus());
    const tooltip = document.querySelector<HTMLElement>("[role='dialog'][aria-label='查看内容信息详情']")!;
    const tooltipContainer = tooltip.closest<HTMLElement>("[data-tooltip-container='true']")!;
    const left = Number.parseFloat(tooltipContainer.style.left);
    const top = Number.parseFloat(tooltipContainer.style.top);
    expect(tooltip.textContent).toContain(longSourceApplication);
    expect(tooltip.textContent).toContain(longSourceProcess);
    expect(left).toBeGreaterThanOrEqual(8);
    expect(left + tooltipContainer.offsetWidth).toBeLessThanOrEqual(window.innerWidth - 8);
    expect(top).toBeGreaterThanOrEqual(8);
    expect(top + tooltipContainer.offsetHeight).toBeLessThanOrEqual(window.innerHeight - 8);
    expect(tooltipContainer.style.getPropertyValue("--tooltip-arrow-left")).not.toBe("");
    expect(primitivesCss).toMatch(/\.hintBubble\s*\{[\s\S]*overflow:\s*visible;/);
    expect(primitivesCss).toMatch(/\.hintBubbleContentInteractive\s*\{[\s\S]*overflow:\s*auto;/);
    expect(primitivesCss).toMatch(/\.hintBubbleContent\s*\{[\s\S]*overflow-wrap:\s*anywhere;/);
    expect(primitivesCss).toMatch(/\.hintBubbleInteractive\s*\{[\s\S]*pointer-events:\s*auto;/);
    expect(tooltip.scrollHeight).toBeGreaterThan(tooltip.clientHeight);

    await act(async () => {
      hint.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    });
    expect(document.activeElement).toBe(tooltip);
    tooltip.scrollTop = 120;
    expect(tooltip.scrollTop).toBe(120);

    Object.defineProperty(hint, "getBoundingClientRect", {
      configurable: true,
      value: () => ({
        left: 292,
        right: 310,
        top: 420,
        bottom: 438,
        width: 18,
        height: 18,
        x: 292,
        y: 420,
        toJSON: () => undefined
      })
    });
    await act(async () => window.dispatchEvent(new Event("scroll")));
    expect(Number.parseFloat(tooltipContainer.style.left)).toBe(10);
    expect(Number.parseFloat(tooltipContainer.style.top)).toBe(132);

    await act(async () => {
      tooltip.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });
    expect(document.activeElement).toBe(hint);
    expect(document.querySelector("[role='dialog'][aria-label='查看内容信息详情']")).toBeNull();
    expect(hint.getAttribute("aria-expanded")).toBe("false");
    expect(hint.hasAttribute("aria-controls")).toBe(false);
  });

  it("opens and closes the details tooltip on pointer hover", async () => {
    vi.useFakeTimers();
    await render(readyState());
    const hint = document.querySelector<HTMLElement>("[aria-label='查看内容信息']")!;
    Object.defineProperty(hint, "getBoundingClientRect", {
      configurable: true,
      value: () => ({
        left: 280,
        right: 298,
        top: 200,
        bottom: 218,
        width: 18,
        height: 18,
        x: 280,
        y: 200,
        toJSON: () => undefined
      })
    });
    await act(async () => {
      hint.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
    });
    expect(document.querySelector("[role='dialog'][aria-label='查看内容信息详情']")).toBeTruthy();
    await act(async () => {
      hint.dispatchEvent(new MouseEvent("mouseout", { bubbles: true }));
      vi.advanceTimersByTime(101);
    });
    expect(document.querySelector("[role='dialog'][aria-label='查看内容信息详情']")).toBeNull();
  });
});
