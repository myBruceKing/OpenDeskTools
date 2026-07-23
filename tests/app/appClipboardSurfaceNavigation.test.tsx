// @vitest-environment jsdom

import { act } from "react";
import type { ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  closeSurface: vi.fn<() => Promise<boolean>>(),
  listen: vi.fn(),
  useClipboardController: vi.fn()
}));

vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));
vi.mock("../../src/app/useClipboardController", () => ({
  useClipboardController: (...args: unknown[]) => {
    mocks.useClipboardController(...args);
    return {
      state: {},
      loadImage: vi.fn(),
      loadSourceIcon: vi.fn(),
      updateText: vi.fn(),
      closeSurface: mocks.closeSurface,
      setFavorite: vi.fn(),
      deleteItem: vi.fn(),
      clearUnfavoriteHistory: vi.fn()
    };
  }
}));
vi.mock("../../src/app/overviewClient", () => ({
  overviewClient: {
    load: vi.fn(async () => ({ version: "0.1.0", startupEnabled: false, serviceState: "running" })),
    subscribeToUsageChanges: vi.fn(async () => () => undefined)
  }
}));
vi.mock("../../src/app/themeRuntime", () => ({
  createThemeRootPresentation: vi.fn(() => ({})),
  useDocumentTheme: vi.fn(),
  useSystemThemePreferences: vi.fn(() => ({ systemDark: false, systemReducedMotion: false }))
}));
vi.mock("../../src/app/useThemeController", () => ({
  useThemeController: () => ({ state: { current: {} }, update: vi.fn() })
}));
vi.mock("../../src/app/useDesktopWebViewGuards", () => ({ useDesktopWebViewGuards: vi.fn() }));
vi.mock("../../src/components/shell/AppShell", () => ({
  AppShell: ({ activeRoute, onNavigate, children }: {
    activeRoute: string;
    onNavigate: (route: "clipboard" | "general" | "overview") => void;
    children: unknown;
  }) => (
    <div data-active-route={activeRoute}>
      <button type="button" onClick={() => onNavigate("clipboard")}>剪贴板</button>
      <button type="button" onClick={() => onNavigate("general")}>常规</button>
      <button type="button" onClick={() => onNavigate("overview")}>概览</button>
      {children as ReactNode}
    </div>
  )
}));
vi.mock("../../src/pages/clipboard/ClipboardPage", () => ({ ClipboardPage: () => <div>剪贴板管理页</div> }));
vi.mock("../../src/pages/capture-qr/CaptureQrPage", () => ({ CaptureQrPage: () => null }));
vi.mock("../../src/pages/general/GeneralPage", () => ({ GeneralPage: () => <div>常规页</div> }));
vi.mock("../../src/pages/hotkeys/HotkeysPage", () => ({ HotkeysPage: () => null }));
vi.mock("../../src/pages/overview/OverviewPage", () => ({ OverviewPage: () => <div>概览页</div> }));
vi.mock("../../src/pages/quick-launch/QuickLaunchPage", () => ({ QuickLaunchPage: () => null }));
vi.mock("../../src/pages/theme/ThemePage", () => ({ ThemePage: () => null }));

import App from "../../src/app/App";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

describe("App clipboard surface isolation", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(async () => {
    window.history.replaceState(null, "", "#overview");
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
    await act(async () => root.render(<App />));
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    document.body.replaceChildren();
    vi.clearAllMocks();
  });

  it("keeps the main window on its current page when a clipboard hotkey event is emitted", async () => {
    await act(async () => {
      window.dispatchEvent(new CustomEvent("hotkey://action", {
        detail: {
          actionId: "clipboard.open_panel",
          phase: "pressed",
          timestampMs: 100,
          registrationRevision: 3
        }
      }));
    });

    expect(document.querySelector("[data-active-route]")?.getAttribute("data-active-route")).toBe("overview");
    expect(window.location.hash).toBe("#overview");
    expect(mocks.listen).not.toHaveBeenCalledWith("qr://conversion-result", expect.any(Function));
  });

  it("opens the management page only through normal navigation and leaves without surface cleanup", async () => {
    const buttons = document.querySelectorAll<HTMLButtonElement>("button");
    await act(async () => buttons[0].click());
    expect(document.body.textContent).toContain("剪贴板管理页");
    expect(mocks.useClipboardController).toHaveBeenCalledWith();

    await act(async () => buttons[1].click());
    expect(document.querySelector("[data-active-route]")?.getAttribute("data-active-route")).toBe("general");
    expect(mocks.closeSurface).not.toHaveBeenCalled();
  });
});
