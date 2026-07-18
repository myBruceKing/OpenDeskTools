// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  createClipboardLoadingState,
  createClipboardReadyViewModel,
  type ClipboardControllerState,
  type ClipboardHistoryItem
} from "../../src/app/clipboardModel";
import { ClipboardPage } from "../../src/pages/clipboard/ClipboardPage";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const rawItems: ClipboardHistoryItem[] = [
  {
    id: "1",
    kind: "text",
    textContent: "普通内容",
    sourceApplication: null,
    sourceProcess: null,
    capturedAtMs: 1_720_000_000_000,
    byteSize: 12,
    isFavorite: false
  },
  {
    id: "2",
    kind: "text",
    textContent: "收藏内容",
    sourceApplication: "记事本",
    sourceProcess: "notepad.exe",
    capturedAtMs: 1_720_000_001_000,
    byteSize: 12,
    isFavorite: true
  }
];

function readyState(overrides: Partial<ClipboardControllerState> = {}): ClipboardControllerState {
  return {
    status: "ready",
    viewModel: createClipboardReadyViewModel({ items: rawItems, totalCount: 2 }),
    error: null,
    pendingItemIds: [],
    clearing: false,
    ...overrides
  };
}

describe("ClipboardPage", () => {
  let host: HTMLDivElement;
  let root: Root;
  const onSetFavorite = vi.fn();
  const onDelete = vi.fn();
  const onClear = vi.fn();

  beforeEach(() => {
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true,
      value: vi.fn()
    });
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    document.body.replaceChildren();
    vi.clearAllMocks();
  });

  async function render(state: ClipboardControllerState) {
    await act(async () => {
      root.render(
        <ClipboardPage
          state={state}
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
    const favoriteButton = document.querySelector<HTMLButtonElement>("button[aria-label='收藏']")!;
    expect(favoriteButton).toBeTruthy();
    await act(async () => favoriteButton.click());

    expect(onSetFavorite).toHaveBeenCalledWith("1", true);
    expect(document.querySelector("button[aria-label='收藏']")).toBeTruthy();
    expect(document.querySelectorAll("button[aria-label='取消收藏']")).toHaveLength(1);
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

  it("confirms backend delete and clear operations with permanent-action copy", async () => {
    await render(readyState());
    const deleteButton = Array.from(document.querySelectorAll<HTMLButtonElement>("button"))
      .find((button) => button.textContent?.trim() === "删除")!;
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
      viewModel: createClipboardReadyViewModel({ items: [...items], totalCount: items.length })
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
});
