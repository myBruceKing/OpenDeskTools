import { describe, expect, it } from "vitest";
import { GLOBAL_HOTKEY_DEFINITIONS } from "../../src/app/hotkeyModel";
import {
  createOverviewViewModel,
  EMPTY_OVERVIEW_VIEW_MODEL,
  getToolWheelShortcutLabel,
  type OverviewBackendSnapshot
} from "../../src/app/overviewModel";

const EMPTY_RUNTIME = {
  binding: null,
  enabled: null,
  state: "unavailable",
  detail: null
} as const;

function snapshot(overrides: Partial<OverviewBackendSnapshot> = {}): OverviewBackendSnapshot {
  return {
    version: "0.1.0",
    serviceState: "running",
    startupEnabled: false,
    hotkeys: [],
    statistics: null,
    ...overrides
  };
}

describe("createOverviewViewModel", () => {
  it("keeps presentation definitions but exposes unavailable runtime fields without a source", () => {
    expect(EMPTY_OVERVIEW_VIEW_MODEL.sourceAvailable).toBe(false);
    expect(EMPTY_OVERVIEW_VIEW_MODEL.hotkeys).toHaveLength(GLOBAL_HOTKEY_DEFINITIONS.length);
    expect(EMPTY_OVERVIEW_VIEW_MODEL.hotkeys).toEqual(
      GLOBAL_HOTKEY_DEFINITIONS.map(({ id, title, description }) => ({
        id,
        title,
        description,
        ...EMPTY_RUNTIME
      }))
    );
    expect(EMPTY_OVERVIEW_VIEW_MODEL.statistics).toEqual({
      todayTriggers: null,
      weekTriggers: null,
      monthTriggers: null,
      savedMinutesThisMonth: null
    });
  });

  it("does not invent bindings or states when the backend omits hotkeys", () => {
    const viewModel = createOverviewViewModel(snapshot({ hotkeys: null }));

    expect(viewModel.sourceAvailable).toBe(true);
    expect(viewModel.hotkeys).toHaveLength(GLOBAL_HOTKEY_DEFINITIONS.length);
    expect(viewModel.hotkeys.every((hotkey) => hotkey.binding === null)).toBe(true);
    expect(viewModel.hotkeys.every((hotkey) => hotkey.enabled === null)).toBe(true);
    expect(viewModel.hotkeys.every((hotkey) => hotkey.state === "unavailable")).toBe(true);
    expect(viewModel.hotkeys.every((hotkey) => hotkey.detail === null)).toBe(true);
  });

  it("keeps all definitions unavailable when the backend returns an empty hotkey list", () => {
    const viewModel = createOverviewViewModel(snapshot({ hotkeys: [] }));

    expect(viewModel.hotkeys).toHaveLength(GLOBAL_HOTKEY_DEFINITIONS.length);
    expect(viewModel.hotkeys.every((hotkey) => hotkey.state === "unavailable")).toBe(true);
  });

  it("maps known runtime values without dropping definitions from a partial backend response", () => {
    const viewModel = createOverviewViewModel(
      snapshot({
        hotkeys: [
          { id: "unknown", binding: "F9", enabled: true, state: "normal", detail: null },
          { id: "capture", binding: "Ctrl+F1", enabled: false, state: "conflict", detail: "occupied" }
        ]
      })
    );

    expect(viewModel.hotkeys).toHaveLength(GLOBAL_HOTKEY_DEFINITIONS.length);
    expect(viewModel.hotkeys.find((hotkey) => hotkey.id === "capture")).toEqual(
      expect.objectContaining({
        binding: "Ctrl+F1",
        enabled: false,
        state: "conflict",
        detail: "occupied"
      })
    );
    expect(viewModel.hotkeys.find((hotkey) => hotkey.id === "toolWheel")).toEqual(
      expect.objectContaining({ id: "toolWheel", ...EMPTY_RUNTIME })
    );
    expect(viewModel.hotkeys.map((hotkey) => hotkey.id)).not.toContain("unknown");
  });

  it("marks null runtime values unavailable instead of applying defaults", () => {
    const viewModel = createOverviewViewModel(
      snapshot({
        hotkeys: [{ id: "clipboardPanel", binding: null, enabled: null, state: null, detail: null }]
      })
    );

    expect(viewModel.hotkeys).toHaveLength(GLOBAL_HOTKEY_DEFINITIONS.length);
    expect(viewModel.hotkeys.find((hotkey) => hotkey.id === "clipboardPanel")).toEqual(
      expect.objectContaining({ id: "clipboardPanel", ...EMPTY_RUNTIME })
    );
  });
});

describe("getToolWheelShortcutLabel", () => {
  it("does not claim a default shortcut when runtime binding is unavailable", () => {
    expect(getToolWheelShortcutLabel(EMPTY_OVERVIEW_VIEW_MODEL.hotkeys)).toBe("快捷键服务未接入");
  });

  it("uses the actual tool wheel binding returned by the backend", () => {
    const viewModel = createOverviewViewModel(
      snapshot({
        hotkeys: [{ id: "toolWheel", binding: "Ctrl+Space", enabled: true, state: "normal", detail: null }]
      })
    );

    expect(getToolWheelShortcutLabel(viewModel.hotkeys)).toBe("Ctrl+Space 呼出");
  });
});
