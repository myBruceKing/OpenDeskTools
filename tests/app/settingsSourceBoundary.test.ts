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

  it("routes loaded backend truth into hotkeys and general pages", () => {
    const app = source("src/app/App.tsx");
    const hotkeysPage = source("src/pages/hotkeys/HotkeysPage.tsx");
    const generalPage = source("src/pages/general/GeneralPage.tsx");

    expect(app).toContain("<HotkeysPage onSnapshotChanged={refreshOverview} />");
    expect(hotkeysPage).toContain("useHotkeyController");
    expect(hotkeysPage).toContain("onSnapshotChanged");
    expect(app).toContain("<GeneralPage />");
    expect(hotkeysPage).not.toContain("EMPTY_OVERVIEW_VIEW_MODEL");
    expect(hotkeysPage).not.toContain("OverviewHotkeyViewModel");
    // The general page sources real backend truth (autostart Run key + data
    // directory) through its own controller instead of fake local state.
    expect(generalPage).toContain("useGeneralSettings");
    expect(generalPage).toContain("viewModel.autostartEnabled");
    expect(generalPage).not.toContain("useState");
  });

  it("renders all backend classification branches without pretending unavailable actions registered", () => {
    const hotkeysPage = source("src/pages/hotkeys/HotkeysPage.tsx");

    expect(hotkeysPage).toContain('classification === "system_reserved"');
    expect(hotkeysPage).toContain("强制覆盖系统热键");
    expect(hotkeysPage).toContain("该组合被系统禁止注册，不能保存");
    expect(hotkeysPage).toContain("连续按键序列不支持注册为全局快捷键");
    expect(hotkeysPage).toContain("功能接入后生效");
    expect(hotkeysPage).toContain("当前状态不会显示为已注册");
    expect(hotkeysPage).toContain('action.actionAvailable ? toHotkeyBadgeState(action.runtimeState) : "unavailable"');
    expect(hotkeysPage).toContain(
      "disabled={!canSaveHotkeyEditor(state) || editorActionPending}"
    );
  });

  it("keeps verbose runtime detail in the status tooltip instead of the list row", () => {
    const badge = source("src/components/primitives/Badge.tsx");

    expect(badge).toContain("detail && (");
    expect(badge).toContain("<HintTooltip");
    expect(badge).toContain("状态说明");
    expect(badge).not.toContain("statusDetail");
    expect(badge).not.toContain("<small");
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
