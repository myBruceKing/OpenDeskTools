import { useEffect } from "react";
import type { ResolvedTheme } from "./themeRuntime";

type SetClipboardSurfaceUnderlayColor = (color: string) => Promise<void>;

export function normalizeSurfaceUnderlayColor(value: string) {
  const color = value.trim();
  const shortHex = /^#([0-9a-f]{3})$/i.exec(color);
  if (shortHex) {
    return `#${shortHex[1].split("").map((part) => `${part}${part}`).join("")}`.toUpperCase();
  }
  const hex = /^#([0-9a-f]{6})$/i.exec(color);
  if (hex) return `#${hex[1]}`.toUpperCase();
  const rgb = /^rgb\(\s*(\d{1,3})\s*,\s*(\d{1,3})\s*,\s*(\d{1,3})\s*\)$/i.exec(color)
    ?? /^rgba\(\s*(\d{1,3})\s*,\s*(\d{1,3})\s*,\s*(\d{1,3})\s*,\s*1(?:\.0*)?\s*\)$/i.exec(color);
  if (!rgb) return null;
  const channels = rgb.slice(1, 4).map(Number);
  if (channels.some((channel) => channel > 255)) return null;
  return `#${channels.map((channel) => channel.toString(16).padStart(2, "0")).join("")}`.toUpperCase();
}

export function useClipboardSurfaceUnderlayColor(
  resolvedTheme: ResolvedTheme,
  setUnderlayColor: SetClipboardSurfaceUnderlayColor
) {
  useEffect(() => {
    let disposed = false;
    const frame = window.requestAnimationFrame(() => {
      if (disposed) return;
      const computed = window.getComputedStyle(document.documentElement)
        .getPropertyValue("--border-default");
      const color = normalizeSurfaceUnderlayColor(computed);
      if (!color) {
        console.error("Failed to resolve clipboard surface underlay color", computed);
        return;
      }
      void setUnderlayColor(color).catch((error) => {
        console.error("Failed to synchronize clipboard surface underlay color", error);
      });
    });
    return () => {
      disposed = true;
      window.cancelAnimationFrame(frame);
    };
  }, [resolvedTheme, setUnderlayColor]);
}
