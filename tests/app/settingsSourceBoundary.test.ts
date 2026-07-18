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

  it("keeps capture and theme setting controls disabled without local change handlers", () => {
    const settingsPages = [
      source("src/pages/capture-qr/CaptureQrPage.tsx"),
      source("src/pages/theme/ThemePage.tsx")
    ].join("\n");

    expect(settingsPages).not.toMatch(/onClick=\{\(\) => set/);
    expect(settingsPages).not.toMatch(/onChange=\{\(event\) => set/);
    expect(settingsPages).not.toMatch(/<SwitchRow[^>]*checked(?:\s|\/>)/);
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
});
