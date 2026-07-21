// @vitest-environment jsdom

import { act, useRef } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  collectWindowSurfaceMetrics,
  useWindowSurfaceMetricsTrace
} from "../../src/app/windowSurfaceMetrics";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function rect(left: number, top: number, width: number, height: number): DOMRect {
  return {
    x: left,
    y: top,
    left,
    top,
    right: left + width,
    bottom: top + height,
    width,
    height,
    toJSON: () => ({})
  } as DOMRect;
}

describe("window surface metrics", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    host = document.createElement("div");
    host.id = "root";
    document.body.append(host);
    root = createRoot(host);
    Object.defineProperty(window, "devicePixelRatio", { configurable: true, value: 1.25 });
    Object.defineProperty(window, "innerWidth", { configurable: true, value: 380 });
    Object.defineProperty(window, "innerHeight", { configurable: true, value: 520 });
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      queueMicrotask(() => callback(16));
      return 1;
    });
    vi.spyOn(window, "cancelAnimationFrame").mockImplementation(() => undefined);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    document.body.replaceChildren();
    vi.restoreAllMocks();
  });

  it("serializes viewport, every root rect, and the computed edge contract", () => {
    const windowRoot = document.createElement("div");
    const surface = document.createElement("section");
    surface.dataset.windowBorderLayer = "true";
    surface.style.position = "absolute";
    surface.style.inset = "0px";
    surface.style.boxSizing = "border-box";
    surface.style.border = "1px solid rgb(224, 222, 220)";
    surface.style.backgroundColor = "rgb(255, 255, 255)";
    windowRoot.append(surface);
    host.append(windowRoot);
    windowRoot.getBoundingClientRect = () => rect(0, 0, 380, 520);
    surface.getBoundingClientRect = () => rect(0, 0, 380, 520);

    const metrics = JSON.parse(collectWindowSurfaceMetrics("clipboard", "mounted", windowRoot));
    expect(metrics).toMatchObject({
      surfaceName: "clipboard",
      cause: "mounted",
      devicePixelRatio: 1.25,
      innerWidth: 380,
      innerHeight: 520,
      documentElement: { rect: expect.any(Object), computed: expect.any(Object) },
      body: { rect: expect.any(Object), computed: expect.any(Object) },
      appRoot: { rect: expect.any(Object), computed: expect.any(Object) },
      windowRoot: { rect: { width: 380, height: 520 }, computed: expect.any(Object) },
      surface: {
        rect: { left: 0, top: 0, right: 380, bottom: 520, width: 380, height: 520 },
        computed: {
          position: "absolute",
          boxSizing: "border-box",
          borderTopWidth: "1px",
          borderRightWidth: "1px",
          borderBottomWidth: "1px",
          borderLeftWidth: "1px",
          backgroundColor: "rgb(255, 255, 255)"
        }
      }
    });
  });

  it("writes mounted and resized metrics through the debug trace bridge", async () => {
    const trace = vi.fn(async () => undefined);
    function Harness() {
      const windowRootRef = useRef<HTMLDivElement>(null);
      useWindowSurfaceMetricsTrace("clipboard-preview", windowRootRef, trace);
      return (
        <div ref={windowRootRef}>
          <section data-window-border-layer="true" />
        </div>
      );
    }

    await act(async () => {
      root.render(<Harness />);
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(trace).toHaveBeenCalledWith("surface_metrics", null, expect.stringContaining('"cause":"mounted"'));

    await act(async () => {
      window.dispatchEvent(new Event("resize"));
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(trace).toHaveBeenLastCalledWith(
      "surface_metrics",
      null,
      expect.stringContaining('"cause":"resized"')
    );
  });
});
