// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  load: vi.fn(),
  setToggle: vi.fn(),
  selectAndMigrateDataDirectory: vi.fn()
}));

vi.mock("../../src/app/generalClient", () => ({
  generalClient: {
    load: mocks.load,
    setToggle: mocks.setToggle,
    selectAndMigrateDataDirectory: mocks.selectAndMigrateDataDirectory
  }
}));

import { GeneralPage } from "../../src/pages/general/GeneralPage";

const snapshot = (overrides: Record<string, unknown> = {}) => ({
  version: "0.1.0",
  autostartEnabled: false,
  startMinimized: false,
  closeToTray: true,
  crashDiagnosticsEnabled: false,
  dataDirectory: "C:\\Users\\me\\AppData\\Roaming\\com.opendesktools.app",
  ...overrides
});

let container: HTMLDivElement;
let root: Root;

async function renderPage() {
  await act(async () => {
    root.render(<GeneralPage />);
  });
  // Flush the effect-driven initial load.
  await act(async () => {
    await Promise.resolve();
  });
}

function autostartToggle(): HTMLButtonElement {
  const toggle = container.querySelector<HTMLButtonElement>('[aria-label="开机自启动"]');
  if (!toggle) {
    throw new Error("autostart toggle should be rendered once settings load");
  }
  return toggle;
}

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  mocks.load.mockReset();
  mocks.setToggle.mockReset();
  mocks.selectAndMigrateDataDirectory.mockReset();
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("GeneralPage autostart", () => {
  it("reflects the loaded autostart state and data directory", async () => {
    mocks.load.mockResolvedValue(snapshot({ autostartEnabled: true }));

    await renderPage();

    expect(autostartToggle().getAttribute("aria-checked")).toBe("true");
    expect(
      container.querySelector<HTMLInputElement>('input[value="C:\\\\Users\\\\me\\\\AppData\\\\Roaming\\\\com.opendesktools.app"]')
    ).not.toBeNull();
  });

  it("toggles autostart on and refreshes from the returned snapshot", async () => {
    mocks.load.mockResolvedValue(snapshot({ autostartEnabled: false }));
    mocks.setToggle.mockResolvedValue(snapshot({ autostartEnabled: true }));

    await renderPage();
    expect(autostartToggle().getAttribute("aria-checked")).toBe("false");

    await act(async () => {
      autostartToggle().click();
    });
    await act(async () => {
      await Promise.resolve();
    });

    expect(mocks.setToggle).toHaveBeenCalledWith("autostart", true);
    expect(autostartToggle().getAttribute("aria-checked")).toBe("true");
  });

  it("persists the close-to-tray preference through the toggle client", async () => {
    mocks.load.mockResolvedValue(snapshot({ closeToTray: true }));
    mocks.setToggle.mockResolvedValue(snapshot({ closeToTray: false }));

    await renderPage();

    const toggle = container.querySelector<HTMLButtonElement>('[aria-label="关闭按钮最小化到托盘"]');
    if (!toggle) {
      throw new Error("close-to-tray toggle should render");
    }
    expect(toggle.getAttribute("aria-checked")).toBe("true");

    await act(async () => {
      toggle.click();
    });
    await act(async () => {
      await Promise.resolve();
    });

    expect(mocks.setToggle).toHaveBeenCalledWith("closeToTray", false);
    expect(
      container
        .querySelector<HTMLButtonElement>('[aria-label="关闭按钮最小化到托盘"]')
        ?.getAttribute("aria-checked")
    ).toBe("false");
  });

  it("opens native path selection then reports the scheduled safe restart", async () => {
    mocks.load.mockResolvedValue(snapshot());
    mocks.selectAndMigrateDataDirectory.mockResolvedValue({
      dataDirectory: "D:\\OpenDeskToolsData",
      restartRequired: true
    });

    await renderPage();
    const chooseButton = Array.from(container.querySelectorAll("button")).find(
      (button) => button.textContent === "选择路径"
    );
    if (!chooseButton) throw new Error("path selection button should render");
    await act(async () => chooseButton.click());

    expect(mocks.selectAndMigrateDataDirectory).toHaveBeenCalledTimes(1);
    expect(container.textContent).toContain("应用正在安全重启");
  });

  it("surfaces a failure without flipping the switch", async () => {
    mocks.load.mockResolvedValue(snapshot({ autostartEnabled: false }));
    mocks.setToggle.mockRejectedValue({
      code: "autostart_update_failed",
      message: "开机自启设置未生效：拒绝访问"
    });

    await renderPage();

    await act(async () => {
      autostartToggle().click();
    });
    await act(async () => {
      await Promise.resolve();
    });

    expect(autostartToggle().getAttribute("aria-checked")).toBe("false");
    expect(container.textContent).toContain("开机自启设置未生效：拒绝访问");
  });
});
