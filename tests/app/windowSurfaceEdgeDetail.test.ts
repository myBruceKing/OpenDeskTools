import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const themesCss = readFileSync("src/styles/themes.css", "utf8");
const globalCss = readFileSync("src/styles/global.css", "utf8");
const mainSurfaceCss = readFileSync("src/surfaces/clipboard/ClipboardSurface.module.css", "utf8");
const previewSurfaceCss = readFileSync("src/app/ClipboardPreviewSurfaceRoot.module.css", "utf8");
const mainSource = readFileSync("src/surfaces/clipboard/ClipboardSurface.tsx", "utf8");
const previewSource = readFileSync("src/app/ClipboardPreviewSurfaceRoot.tsx", "utf8");

function themeToken(blockStart: string) {
  const start = themesCss.indexOf(blockStart);
  const end = themesCss.indexOf("\n}", start);
  const block = themesCss.slice(start, end);
  const match = block.match(/--shadow-window-edge-detail:\s*([^;]+);/);
  return match?.[1].replace(/\s+/g, " ").trim() ?? "";
}

describe("window surface edge detail", () => {
  it("uses restrained theme-specific inner light and lowlight tokens", () => {
    const light = themeToken(':root,\n[data-theme="light"]');
    const dark = themeToken('[data-theme="dark"]');
    for (const token of [light, dark]) {
      expect(token).toContain("inset 0 1px 0");
      expect(token).toContain("inset 0 -1px 0");
      expect(token).not.toMatch(/drop-shadow|\binset\s+-?1px\s+0\b/);
    }
    expect(light).toContain("255 255 255 / 82%");
    expect(dark).toContain("255 255 255 / 8%");
  });

  it("paints the shared detail above solid children through a non-interactive overlay", () => {
    const sharedSurfaceRule = globalCss.match(/\[data-window-border-layer="true"\]\s*\{[^}]*\}/)?.[0] ?? "";
    const sharedRule = globalCss.match(/\[data-window-border-layer="true"\]::after\s*\{[^}]*\}/)?.[0] ?? "";
    expect(sharedSurfaceRule).toMatch(/box-sizing:\s*border-box/);
    expect(sharedSurfaceRule).toMatch(/overflow:\s*hidden/);
    expect(sharedSurfaceRule).toMatch(/border:\s*var\(--border-width\) solid var\(--border-default\)/);
    expect(sharedSurfaceRule).toMatch(/border-radius:\s*var\(--radius-window\)/);
    expect(sharedSurfaceRule).toMatch(/background:\s*var\(--surface-window\)/);
    expect(sharedSurfaceRule).toMatch(/color:\s*var\(--text-primary\)/);
    expect(sharedSurfaceRule).not.toMatch(/box-shadow|filter:|drop-shadow|margin:|padding:|transform:/);
    expect(sharedRule).toMatch(/content:\s*""/);
    expect(sharedRule).toMatch(/position:\s*absolute/);
    expect(sharedRule).toMatch(/inset:\s*var\(--border-width\)/);
    expect(sharedRule).toMatch(/z-index:\s*1/);
    expect(sharedRule).toMatch(/border-radius:\s*calc\(var\(--radius-window\) - var\(--border-width\)\)/);
    expect(sharedRule).toMatch(/box-shadow:\s*var\(--shadow-window-edge-detail\)/);
    expect(sharedRule).toMatch(/pointer-events:\s*none/);
    expect(sharedRule).not.toMatch(/filter:|drop-shadow|margin:|padding:|transform:/);
    expect(mainSource).toContain('data-window-border-layer="true"');
    expect(previewSource).toContain('data-window-border-layer="true"');
    expect(mainSurfaceCss).not.toMatch(/--shadow-window-edge-detail|box-shadow:\s*var\(--shadow-window-edge-detail\)/);
    expect(previewSurfaceCss).not.toMatch(/--shadow-window-edge-detail|box-shadow:\s*var\(--shadow-window-edge-detail\)/);
  });

  it("keeps content geometry and the independently spaced side edges free of added side-axis effects", () => {
    const mainRule = mainSurfaceCss.match(/\.surface\s*\{[^}]*\}/)?.[0] ?? "";
    const previewRule = previewSurfaceCss.match(/\.previewSurface\s*\{[^}]*\}/)?.[0] ?? "";
    for (const rule of [mainRule, previewRule]) {
      expect(rule).toMatch(/position:\s*absolute/);
      expect(rule).toMatch(/inset:\s*0/);
      expect(rule).not.toMatch(/border:|border-radius:|overflow:|box-shadow:|clip-path|margin:|padding:/);
    }
  });
});
