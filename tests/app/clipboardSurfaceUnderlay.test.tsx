// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  normalizeSurfaceUnderlayColor,
  useClipboardSurfaceUnderlayColor
} from "../../src/app/clipboardSurfaceUnderlay";
import type { ResolvedTheme } from "../../src/app/themeRuntime";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

describe("clipboard surface native underlay", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      queueMicrotask(() => callback(16));
      return 1;
    });
    vi.spyOn(window, "cancelAnimationFrame").mockImplementation(() => undefined);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    document.documentElement.style.removeProperty("--border-default");
    document.body.replaceChildren();
    vi.restoreAllMocks();
  });

  it("normalizes supported computed colors to strict opaque #RRGGBB", () => {
    expect(normalizeSurfaceUnderlayColor(" #abc ")).toBe("#AABBCC");
    expect(normalizeSurfaceUnderlayColor("#e0dedc")).toBe("#E0DEDC");
    expect(normalizeSurfaceUnderlayColor("rgb(59, 62, 67)")).toBe("#3B3E43");
    expect(normalizeSurfaceUnderlayColor("rgba(59, 62, 67, 1)")).toBe("#3B3E43");
    expect(normalizeSurfaceUnderlayColor("rgba(59, 62, 67, 0.5)")).toBeNull();
    expect(normalizeSurfaceUnderlayColor("rgb(256, 0, 0)")).toBeNull();
    expect(normalizeSurfaceUnderlayColor("transparent")).toBeNull();
  });

  it("synchronizes the resolved light, dark, and system result after CSS variables apply", async () => {
    const setUnderlayColor = vi.fn(async () => undefined);
    function Harness({ resolvedTheme }: { resolvedTheme: ResolvedTheme }) {
      useClipboardSurfaceUnderlayColor(resolvedTheme, setUnderlayColor);
      return null;
    }

    document.documentElement.style.setProperty("--border-default", "#e0dedc");
    await act(async () => {
      root.render(<Harness resolvedTheme="light" />);
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(setUnderlayColor).toHaveBeenLastCalledWith("#E0DEDC");

    document.documentElement.style.setProperty("--border-default", "rgb(59, 62, 67)");
    await act(async () => {
      root.render(<Harness resolvedTheme="dark" />);
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(setUnderlayColor).toHaveBeenLastCalledWith("#3B3E43");
    expect(setUnderlayColor).toHaveBeenCalledTimes(2);
  });

  it("leaves the page running and logs a diagnostic when native synchronization fails", async () => {
    const failure = new Error("underlay unavailable");
    const setUnderlayColor = vi.fn(async () => Promise.reject(failure));
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => undefined);
    function Harness() {
      useClipboardSurfaceUnderlayColor("light", setUnderlayColor);
      return <main>surface remains mounted</main>;
    }
    document.documentElement.style.setProperty("--border-default", "#e0dedc");

    await act(async () => {
      root.render(<Harness />);
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(document.body.textContent).toContain("surface remains mounted");
    expect(consoleError).toHaveBeenCalledWith(
      "Failed to synchronize clipboard surface underlay color",
      failure
    );
  });
});
