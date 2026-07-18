import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { describe, expect, it } from "vitest";

const projectRoot = resolve(import.meta.dirname, "../..");

function source(path: string) {
  return readFileSync(join(projectRoot, path), "utf8");
}

describe("settings production source boundary", () => {
  it.each([
    "src/pages/capture-qr/CaptureQrPage.tsx",
    "src/pages/theme/ThemePage.tsx",
    "src/pages/static/SettingsRows.tsx"
  ])("keeps %s free of local mutable fake settings", (path) => {
    expect(source(path)).not.toContain("useState");
  });

  it("makes SwitchRow unavailable by default and does not mutate locally", () => {
    const settingsRows = source("src/pages/static/SettingsRows.tsx");

    expect(settingsRows).toContain("checked = null");
    expect(settingsRows).not.toContain("setValue");
  });

  it("keeps capture setting controls disabled without local change handlers", () => {
    const settingsPages = source("src/pages/capture-qr/CaptureQrPage.tsx");

    expect(settingsPages).not.toMatch(/onClick=\{\(\) => set/);
    expect(settingsPages).not.toMatch(/onChange=\{\(event\) => set/);
    expect(settingsPages).not.toMatch(/<SwitchRow[^>]*checked(?:\s|\/>)/);
  });

  it("routes ThemeService state into the controlled theme page and root shell", () => {
    const app = source("src/app/App.tsx");
    const shell = source("src/components/shell/AppShell.tsx");

    expect(app).toContain("useThemeController");
    expect(app).toContain("useDocumentTheme(themePresentation)");
    expect(app).toContain("<ThemePage state={themeController.state} onUpdate={themeController.update}");
    expect(shell).not.toContain('data-theme="light"');
    expect(shell).toContain("data-theme={theme.resolvedTheme}");
    expect(shell).toContain("data-accent={theme.accent}");
    expect(shell).toContain("data-reduce-transparency={String(theme.reduceTransparency)}");
    expect(shell).toContain("data-animation-speed={theme.animationSpeed}");
  });

  it("lets body-level dialog portals inherit the same document-root theme", () => {
    const runtime = source("src/app/themeRuntime.ts");

    expect(runtime).toContain("document.documentElement");
    expect(runtime).toContain("applyThemeRootPresentation");
  });

  it("does not hardcode the tool wheel shortcut in the overview page", () => {
    const overviewPage = source("src/pages/overview/OverviewPage.tsx");

    expect(overviewPage).not.toContain("Alt + Space");
    expect(overviewPage).toContain("getToolWheelShortcutLabel");
  });

  it("routes loaded overview truth into hotkeys and general pages", () => {
    const app = source("src/app/App.tsx");
    const hotkeysPage = source("src/pages/hotkeys/HotkeysPage.tsx");
    const generalPage = source("src/pages/general/GeneralPage.tsx");

    expect(app).toContain("<HotkeysPage hotkeys={overview.hotkeys}");
    expect(app).toContain("<GeneralPage version={overview.version} startupEnabled={overview.startupEnabled}");
    expect(hotkeysPage).not.toContain("EMPTY_OVERVIEW_VIEW_MODEL");
    expect(generalPage).toContain("startupEnabled: boolean | null");
    expect(generalPage).toContain("version: string | null");
  });

  it("routes the real overview version into AppShell without a fake fallback", () => {
    const app = source("src/app/App.tsx");
    const shell = source("src/components/shell/AppShell.tsx");

    expect(app).toContain("version={overview.version}");
    expect(shell).toContain("version: string | null");
    expect(shell).toContain('{version ?? "—"}');
    expect(shell).not.toContain("v1.3.0");
  });
});
