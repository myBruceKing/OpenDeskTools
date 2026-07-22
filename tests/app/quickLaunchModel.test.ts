// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { quickLaunchClient, type QuickLaunchSnapshotPayload } from "../../src/app/quickLaunchClient";
import {
  DISCOVERED_QUICK_LAUNCH_APPS,
  PINNED_QUICK_LAUNCH_APPS,
  toToolMenuPreviewItems,
  useQuickLaunchViewModel
} from "../../src/app/quickLaunchModel";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function snapshot(name: string): QuickLaunchSnapshotPayload {
  return {
    pinnedApps: [{
      id: name,
      name,
      path: `C:\\Apps\\${name}.exe`,
      arguments: "",
      workingDirectory: null,
      iconPath: "",
      iconIndex: 0,
      source: "测试",
      visible: true,
      available: true,
      iconAvailable: false
    }],
    discoveredApps: [],
    toolMenu: { layout: "wheel", keepOpenOnKeyRelease: false }
  };
}

describe("quick launch production defaults", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    document.body.replaceChildren();
    vi.restoreAllMocks();
  });

  it("starts empty until the native discovery service provides data", () => {
    expect(PINNED_QUICK_LAUNCH_APPS).toEqual([]);
    expect(DISCOVERED_QUICK_LAUNCH_APPS).toEqual([]);
    expect(toToolMenuPreviewItems([])).toEqual([]);
  });

  it("keeps a pushed mutation snapshot when an older reload resolves later", async () => {
    let resolveReload: ((value: QuickLaunchSnapshotPayload) => void) | undefined;
    vi.spyOn(quickLaunchClient, "getSnapshot").mockImplementation(() => new Promise((resolve) => {
      resolveReload = resolve;
    }));
    let current: ReturnType<typeof useQuickLaunchViewModel> | undefined;
    function Harness() {
      current = useQuickLaunchViewModel();
      return createElement("span", null, current.pinnedApps.map((app) => app.name).join(","));
    }

    await act(async () => {
      root.render(createElement(Harness));
      await Promise.resolve();
    });
    await act(async () => {
      current?.actions.syncSnapshot(snapshot("New"));
    });
    expect(host.textContent).toBe("New");

    await act(async () => {
      resolveReload?.(snapshot("Old"));
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(host.textContent).toBe("New");
  });
});
