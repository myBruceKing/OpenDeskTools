import { describe, expect, it, vi } from "vitest";
import {
  applyThemeRootPresentation,
  createThemeRootPresentation,
  observeMediaQuery,
  readableTextColor,
  resolveTheme
} from "../../src/app/themeRuntime";
import type { ThemeSnapshot } from "../../src/app/themeModel";

const snapshot: ThemeSnapshot = {
  mode: "system",
  accent: "#c7427a",
  animationSpeed: "fast",
  reduceTransparency: true,
  background: null,
  backgroundFit: "cover",
  backgroundDim: 24,
  backgroundBlur: 6,
  panelOpacity: 86,
  revision: 2
};

describe("theme runtime", () => {
  it("resolves system and explicit light/dark modes", () => {
    expect(resolveTheme(snapshot, false)).toBe("light");
    expect(resolveTheme(snapshot, true)).toBe("dark");
    expect(resolveTheme({ ...snapshot, mode: "light" }, true)).toBe("light");
    expect(resolveTheme({ ...snapshot, mode: "dark" }, false)).toBe("dark");
  });

  it("builds all root theme values and lets reduced motion override speed", () => {
    expect(createThemeRootPresentation(snapshot, true, false)).toEqual({
      resolvedTheme: "dark",
      accent: "#c7427a",
      accentText: "#ffffff",
      reduceTransparency: true,
      animationSpeed: "fast",
      reducedMotion: false,
      background: null,
      backgroundFit: "cover",
      backgroundDim: 24,
      backgroundBlur: 6,
      panelOpacity: 86,
      backgroundUrl: null
    });
    expect(createThemeRootPresentation(snapshot, false, true)).toMatchObject({
      resolvedTheme: "light",
      animationSpeed: "reduced",
      reducedMotion: true
    });
  });

  it("chooses readable text for dark and light custom accents", () => {
    expect(readableTextColor("#216bd9")).toBe("#ffffff");
    expect(readableTextColor("#f4e04d")).toBe("#171717");
    expect(readableTextColor("invalid")).toBe("#ffffff");
  });

  it("keeps managed background metadata and the main-window object URL in presentation", () => {
    const background = {
      id: "b".repeat(64),
      fileName: "quiet-garden.png",
      byteSize: 4096,
      width: 1600,
      height: 900
    };
    expect(
      createThemeRootPresentation(
        {
          ...snapshot,
          background,
          backgroundFit: "contain",
          backgroundDim: 36,
          backgroundBlur: 10,
          panelOpacity: 78
        },
        true,
        false,
        "blob:theme-background"
      )
    ).toMatchObject({
      background,
      backgroundFit: "contain",
      backgroundDim: 36,
      backgroundBlur: 10,
      panelOpacity: 78,
      backgroundUrl: "blob:theme-background"
    });
  });

  it("observes a media query immediately and removes the same change listener", () => {
    let changeListener: ((event: { matches: boolean }) => void) | undefined;
    const addEventListener = vi.fn((_type, listener) => {
      changeListener = listener;
    });
    const removeEventListener = vi.fn();
    const listener = vi.fn();
    const stop = observeMediaQuery(
      () => ({ matches: true, addEventListener, removeEventListener }),
      "(prefers-color-scheme: dark)",
      listener
    );

    expect(listener).toHaveBeenCalledWith(true);
    changeListener?.({ matches: false });
    expect(listener).toHaveBeenLastCalledWith(false);
    stop();
    expect(removeEventListener).toHaveBeenCalledWith("change", changeListener);
  });

  it("applies theme data and accent to the document root so portals inherit it", () => {
    const attributes = new Map<string, string>([["data-theme", "legacy"]]);
    const properties = new Map<string, string>([
      ["--accent-primary", "#000000"],
      ["--text-on-accent", "#eeeeee"]
    ]);
    const target = {
      getAttribute: (name: string) => attributes.get(name) ?? null,
      setAttribute: (name: string, value: string) => attributes.set(name, value),
      removeAttribute: (name: string) => attributes.delete(name),
      style: {
        getPropertyValue: (name: string) => properties.get(name) ?? "",
        setProperty: (name: string, value: string) => properties.set(name, value),
        removeProperty: (name: string) => {
          const previous = properties.get(name) ?? "";
          properties.delete(name);
          return previous;
        }
      }
    };

    const cleanup = applyThemeRootPresentation(
      target,
      createThemeRootPresentation(snapshot, true, false)
    );
    expect(Object.fromEntries(attributes)).toMatchObject({
      "data-theme": "dark",
      "data-accent": "#c7427a",
      "data-reduce-transparency": "true",
      "data-animation-speed": "fast",
      "data-reduced-motion": "false"
    });
    expect(properties.get("--accent-primary")).toBe("#c7427a");
    expect(properties.get("--text-on-accent")).toBe("#ffffff");

    cleanup();
    expect(attributes.get("data-theme")).toBe("legacy");
    expect(attributes.has("data-accent")).toBe(false);
    expect(properties.get("--accent-primary")).toBe("#000000");
    expect(properties.get("--text-on-accent")).toBe("#eeeeee");
  });

  it("does not clobber a newer document-root owner during cleanup", () => {
    const attributes = new Map<string, string>();
    const properties = new Map<string, string>();
    const target = {
      getAttribute: (name: string) => attributes.get(name) ?? null,
      setAttribute: (name: string, value: string) => attributes.set(name, value),
      removeAttribute: (name: string) => attributes.delete(name),
      style: {
        getPropertyValue: (name: string) => properties.get(name) ?? "",
        setProperty: (name: string, value: string) => properties.set(name, value),
        removeProperty: (name: string) => {
          const previous = properties.get(name) ?? "";
          properties.delete(name);
          return previous;
        }
      }
    };
    const cleanup = applyThemeRootPresentation(
      target,
      createThemeRootPresentation(snapshot, true, false)
    );

    attributes.set("data-theme", "light");
    properties.set("--accent-primary", "#7955c7");
    properties.set("--text-on-accent", "#123456");
    cleanup();

    expect(attributes.get("data-theme")).toBe("light");
    expect(properties.get("--accent-primary")).toBe("#7955c7");
    expect(properties.get("--text-on-accent")).toBe("#123456");
  });
});
