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

const longSourceApplication = "超长应用".repeat(64);
const longSourceProcess = `C:\\${"deep-folder\\".repeat(60)}app.exe`.slice(0, 512);

function readyState(overrides: Partial<ClipboardControllerState> = {}): ClipboardControllerState {
  return {
    status: "ready",
    viewModel: createClipboardReadyViewModel({ items: rawItems, totalCount: 2, monitoring: "running" }),
    error: null,
    realtimeError: null,
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
      viewModel: createClipboardReadyViewModel({
        items: [...items],
        totalCount: items.length,
        monitoring: "running"
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
        monitoring: "running"
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
        monitoring: "running"
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
