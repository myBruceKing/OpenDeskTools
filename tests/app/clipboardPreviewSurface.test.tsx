// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { readFileSync } from "node:fs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createClipboardReadyViewModel, type ClipboardControllerState } from "../../src/app/clipboardModel";

const previewMocks = vi.hoisted(() => ({
  getState: vi.fn(),
  subscribe: vi.fn(),
  publishHover: vi.fn(),
  traceDebug: vi.fn(),
  setUnderlayColor: vi.fn(),
  listener: undefined as ((change: {
    change: "opened" | "selection_changed" | "closed";
    recordId: string | null;
    visible: boolean;
  }) => void) | undefined,
  loadImage: vi.fn()
}));

vi.mock("../../src/app/clipboardClient", () => ({
  clipboardClient: {
    getPreviewSurfaceState: previewMocks.getState,
    subscribePreviewSurface: previewMocks.subscribe,
    publishPreviewHover: previewMocks.publishHover,
    tracePreviewDebug: previewMocks.traceDebug,
    setSurfaceUnderlayColor: previewMocks.setUnderlayColor
  }
}));

vi.mock("../../src/app/useClipboardController", () => ({
  useClipboardController: () => ({
    state: previewState,
    loadImage: previewMocks.loadImage
  })
}));

vi.mock("../../src/app/themeRuntime", () => ({
  createThemeRootPresentation: () => ({ resolvedTheme: "light" }),
  useDocumentTheme: () => undefined,
  useSystemThemePreferences: () => ({ systemDark: false, systemReducedMotion: false })
}));

vi.mock("../../src/app/useThemeController", () => ({
  useThemeController: () => ({ state: { current: {} } })
}));

vi.mock("../../src/app/useDesktopWebViewGuards", () => ({
  useDesktopWebViewGuards: () => undefined
}));

import { ClipboardPreviewSurfaceRoot } from "../../src/app/ClipboardPreviewSurfaceRoot";

const rawItems = [
  {
    id: "1", revision: 1, kind: "text" as const, textContent: "独立窗口中的长文本预览",
    sourceApplication: "记事本", sourceProcess: "notepad.exe", capturedAtMs: 1_720_000_000_000,
    byteSize: 32, isFavorite: false, sourceIconAvailable: false, fileCount: null, fileNames: null,
    displayCategory: "text" as const
  },
  {
    id: "2", revision: 1, kind: "image" as const, textContent: null,
    sourceApplication: "截图工具", sourceProcess: "capture.exe", capturedAtMs: 1_720_000_001_000,
    byteSize: 1024, isFavorite: false, sourceIconAvailable: false, fileCount: null, fileNames: null,
    displayCategory: "image" as const
  }
];

const previewState: ClipboardControllerState = {
  status: "ready",
  viewModel: createClipboardReadyViewModel({
    items: rawItems,
    totalCount: rawItems.length,
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

const previewCss = readFileSync("src/app/ClipboardPreviewSurfaceRoot.module.css", "utf8");
const surfaceRootCss = readFileSync("src/app/ClipboardSurfaceRoot.module.css", "utf8");
const globalCss = readFileSync("src/styles/global.css", "utf8");
const mainSource = readFileSync("src/main.tsx", "utf8");

describe("ClipboardPreviewSurfaceRoot", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
    previewMocks.listener = undefined;
    previewMocks.getState.mockResolvedValue({ recordId: "1", visible: true });
    previewMocks.subscribe.mockImplementation(async (listener) => {
      previewMocks.listener = listener;
      return () => undefined;
    });
    previewMocks.publishHover.mockResolvedValue(undefined);
    previewMocks.traceDebug.mockResolvedValue(undefined);
    previewMocks.setUnderlayColor.mockResolvedValue(undefined);
    previewMocks.loadImage.mockResolvedValue(new Blob([new Uint8Array([1])], { type: "image/png" }));
    Object.defineProperty(URL, "createObjectURL", { configurable: true, value: vi.fn(() => "blob:preview") });
    Object.defineProperty(URL, "revokeObjectURL", { configurable: true, value: vi.fn() });
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    document.body.replaceChildren();
    vi.clearAllMocks();
  });

  async function render() {
    await act(async () => {
      root.render(<ClipboardPreviewSurfaceRoot />);
      await Promise.resolve();
      await Promise.resolve();
    });
  }

  it("hydrates from the state query after subscribing and renders only shared preview content", async () => {
    await render();
    expect(previewMocks.subscribe.mock.invocationCallOrder[0])
      .toBeLessThan(previewMocks.getState.mock.invocationCallOrder[0]);
    expect(document.querySelector("section[aria-label^='剪贴板预览：']")?.textContent).toContain("文本预览");
    expect(document.querySelector("[data-clipboard-history-preview-content='true']")?.textContent)
      .toContain("独立窗口中的长文本预览");
    expect(document.body.textContent).toContain("记事本");
    expect(document.querySelector("[role='radiogroup']")).toBeNull();
    expect(document.querySelector("[role='listbox']")).toBeNull();
    expect(document.querySelector("[data-clipboard-history-actions='true']")).toBeNull();
  });

  it("switches records from the native event and exposes missing/deleted records", async () => {
    await render();
    await act(async () => {
      previewMocks.listener?.({ change: "selection_changed", recordId: "2", visible: true });
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(document.body.textContent).toContain("图片预览");
    expect(previewMocks.loadImage).toHaveBeenCalledWith("2");

    await act(async () => previewMocks.listener?.({
      change: "selection_changed", recordId: "missing", visible: true
    }));
    expect(document.querySelector("[role='alert']")?.textContent).toContain("记录已不可用");
  });

  it("does not let a late initial query overwrite a newer native selection event", async () => {
    let resolveState!: (value: { recordId: string | null; visible: boolean }) => void;
    previewMocks.getState.mockImplementation(() => new Promise((resolve) => {
      resolveState = resolve;
    }));
    await render();
    await act(async () => previewMocks.listener?.({
      change: "selection_changed", recordId: "2", visible: true
    }));
    await act(async () => {
      resolveState({ recordId: "1", visible: true });
      await Promise.resolve();
    });
    expect(document.body.textContent).toContain("图片预览");
    expect(document.body.textContent).not.toContain("独立窗口中的长文本预览");
  });

  it("publishes pointer transitions so the popup can apply its cross-window close buffer", async () => {
    await render();
    const surface = document.querySelector<HTMLElement>("section[aria-label^='剪贴板预览：']")!;
    await act(async () => surface.dispatchEvent(new MouseEvent("pointerover", { bubbles: true })));
    await act(async () => surface.dispatchEvent(new MouseEvent("pointerout", { bubbles: true })));
    expect(previewMocks.publishHover).toHaveBeenNthCalledWith(1, { inside: true, recordId: "1" });
    expect(previewMocks.publishHover).toHaveBeenNthCalledWith(2, { inside: false, recordId: "1" });
  });

  it("uses one border-box transparent root and one symmetric tokenized rounded border", async () => {
    await render();
    const rootRule = previewCss.match(/\.windowRoot\s*\{[^}]*\}/)?.[0] ?? "";
    const surfaceRule = previewCss.match(/\.previewSurface\s*\{[^}]*\}/)?.[0] ?? "";
    const sharedSurfaceRule = globalCss.match(/\[data-window-border-layer="true"\]\s*\{[^}]*\}/)?.[0] ?? "";
    expect(rootRule).toMatch(/position:\s*relative/);
    expect(rootRule).toMatch(/box-sizing:\s*border-box/);
    expect(rootRule).toMatch(/background:\s*var\(--border-default\);/);
    expect(rootRule).not.toMatch(/border:|clip-path:/);
    expect(surfaceRule).toMatch(/position:\s*absolute/);
    expect(surfaceRule).toMatch(/inset:\s*0/);
    expect(surfaceRule).not.toMatch(/(?:width|height):\s*100%/);
    expect(surfaceRule).not.toMatch(/overflow:|border:|border-radius:|clip-path|box-shadow/);
    expect(sharedSurfaceRule).toMatch(/box-sizing:\s*border-box/);
    expect(sharedSurfaceRule).toMatch(/overflow:\s*hidden/);
    expect(sharedSurfaceRule).toMatch(/border:\s*var\(--border-width\) solid var\(--border-default\)/);
    expect(sharedSurfaceRule).toMatch(/border-radius:\s*var\(--radius-window\)/);
    expect(document.querySelectorAll("[data-window-border-layer='true']")).toHaveLength(1);
    expect(document.querySelector("[data-window-border-layer='true']"))
      .toBe(document.querySelector("section[aria-label^='剪贴板预览：']"));
    expect(previewCss).not.toMatch(/#000|black|box-shadow/);
    expect(surfaceRootCss).not.toMatch(/padding:\s*1px|border:|clip-path/);
    expect(globalCss).toMatch(/data-window-surface="clipboard-preview"[\s\S]*background:\s*var\(--border-default\)/);
    expect(mainSource).toContain('window.location.hash === "#clipboard-preview-surface"');
    expect(mainSource).toContain("<ClipboardPreviewSurfaceRoot />");
  });
});
