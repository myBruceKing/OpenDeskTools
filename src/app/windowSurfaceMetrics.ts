import { useEffect, type RefObject } from "react";
import type { ClipboardPreviewDebugEvent } from "./clipboardClient";

type MetricTrace = (
  event: ClipboardPreviewDebugEvent,
  recordId?: string | null,
  detail?: string | null
) => Promise<void>;

function elementMetrics(element: HTMLElement | null) {
  if (!element) return null;
  const rect = element.getBoundingClientRect();
  const style = window.getComputedStyle(element);
  return {
    rect: {
      x: rect.x,
      y: rect.y,
      top: rect.top,
      right: rect.right,
      bottom: rect.bottom,
      left: rect.left,
      width: rect.width,
      height: rect.height
    },
    client: { width: element.clientWidth, height: element.clientHeight },
    offset: { width: element.offsetWidth, height: element.offsetHeight },
    computed: {
      position: style.position,
      inset: `${style.top} ${style.right} ${style.bottom} ${style.left}`,
      boxSizing: style.boxSizing,
      width: style.width,
      height: style.height,
      borderTopWidth: style.borderTopWidth,
      borderRightWidth: style.borderRightWidth,
      borderBottomWidth: style.borderBottomWidth,
      borderLeftWidth: style.borderLeftWidth,
      borderRadius: style.borderRadius,
      backgroundColor: style.backgroundColor,
      zoom: style.getPropertyValue("zoom") || "normal"
    }
  };
}

export function collectWindowSurfaceMetrics(
  surfaceName: "clipboard" | "clipboard-preview",
  cause: "mounted" | "resized",
  windowRoot: HTMLElement | null
) {
  const appRoot = document.getElementById("root");
  const surface = windowRoot?.querySelector<HTMLElement>("[data-window-border-layer='true']") ?? null;
  return JSON.stringify({
    surfaceName,
    cause,
    devicePixelRatio: window.devicePixelRatio,
    innerWidth: window.innerWidth,
    innerHeight: window.innerHeight,
    visualViewportScale: window.visualViewport?.scale ?? null,
    documentElement: elementMetrics(document.documentElement),
    body: elementMetrics(document.body),
    appRoot: elementMetrics(appRoot),
    windowRoot: elementMetrics(windowRoot),
    surface: elementMetrics(surface)
  });
}

export function useWindowSurfaceMetricsTrace(
  surfaceName: "clipboard" | "clipboard-preview",
  windowRootRef: RefObject<HTMLDivElement>,
  trace: MetricTrace
) {
  useEffect(() => {
    let frame = 0;
    const emit = (cause: "mounted" | "resized") => {
      const detail = collectWindowSurfaceMetrics(surfaceName, cause, windowRootRef.current);
      void trace("surface_metrics", null, detail).catch((error) => {
        console.error("Failed to write window surface metrics trace", error);
      });
    };
    const schedule = (cause: "mounted" | "resized") => {
      window.cancelAnimationFrame(frame);
      frame = window.requestAnimationFrame(() => emit(cause));
    };
    schedule("mounted");
    const handleResize = () => schedule("resized");
    window.addEventListener("resize", handleResize);
    return () => {
      window.cancelAnimationFrame(frame);
      window.removeEventListener("resize", handleResize);
    };
  }, [surfaceName, trace, windowRootRef]);
}
