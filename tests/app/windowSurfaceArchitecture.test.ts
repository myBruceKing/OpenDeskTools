import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const surfaceRoots = [
  "src/app/ClipboardSurfaceRoot.tsx",
  "src/app/ClipboardPreviewSurfaceRoot.tsx",
  "src/app/ToolMenuSurfaceRoot.tsx",
  "src/app/QrToastSurfaceRoot.tsx"
];

const clipboardRootCss = readFileSync("src/app/ClipboardSurfaceRoot.module.css", "utf8");
const clipboardPreviewRootCss = readFileSync("src/app/ClipboardPreviewSurfaceRoot.module.css", "utf8");
const toolMenuRootCss = readFileSync("src/app/ToolMenuSurfaceRoot.module.css", "utf8");
const qrToastCss = readFileSync("src/components/patterns/QrConversionToast.module.css", "utf8");
const qrToastRoot = readFileSync("src/app/QrToastSurfaceRoot.tsx", "utf8");

describe("window surface architecture", () => {
  it("uses one shared runtime hook for every independent WebView surface", () => {
    for (const path of surfaceRoots) {
      const source = readFileSync(path, "utf8");
      expect(source).toContain("useWindowSurfaceRuntime");
      expect(source).not.toContain("createThemeRootPresentation");
      expect(source).not.toContain("useDesktopWebViewGuards");
    }
  });

  it("composes shared window-root variants instead of duplicating their declarations", () => {
    expect(clipboardRootCss).toMatch(
      /composes:\s*underlayWindowRoot from "\.\.\/styles\/windowSurfaces\.module\.css"/
    );
    expect(clipboardPreviewRootCss).toMatch(
      /composes:\s*underlayWindowRoot from "\.\.\/styles\/windowSurfaces\.module\.css"/
    );
    expect(toolMenuRootCss).toMatch(
      /composes:\s*centeredTransparentWindowRoot from "\.\.\/styles\/windowSurfaces\.module\.css"/
    );

    for (const css of [clipboardRootCss, clipboardPreviewRootCss, toolMenuRootCss]) {
      expect(css).not.toMatch(/width:\s*100vw|height:\s*100vh|box-sizing:\s*border-box/);
    }
  });

  it("selects the QR surface presentation through the component API", () => {
    expect(qrToastRoot).toContain('presentation="surface"');
    expect(qrToastCss).toContain(".surfaceToast");
    expect(qrToastCss).not.toContain(":global(");
  });
});
