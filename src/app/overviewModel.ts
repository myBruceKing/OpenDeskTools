import {
  GLOBAL_HOTKEY_DEFINITIONS,
  type GlobalHotkeyId,
  type HotkeyState
} from "./hotkeyModel";

export type ServiceState = "running" | "starting" | "stopped" | "error" | "unknown";
export type { HotkeyState };

export type OverviewBackendHotkey = {
  id: string;
  binding: string | null;
  enabled: boolean | null;
  state: Exclude<HotkeyState, "unknown"> | null;
  detail: string | null;
};

export type OverviewBackendSnapshot = {
  version: string;
  serviceState: Exclude<ServiceState, "unknown"> | null;
  startupEnabled: boolean | null;
  hotkeys: OverviewBackendHotkey[] | null;
  statistics: {
    todayTriggers: number | null;
    weekTriggers: number | null;
    monthTriggers: number | null;
    savedMinutesThisMonth: number | null;
  } | null;
};

export type OverviewHotkeyId = GlobalHotkeyId;

export type OverviewHotkeyViewModel = {
  id: OverviewHotkeyId;
  title: string;
  description: string;
  binding: string | null;
  enabled: boolean | null;
  state: HotkeyState;
  detail: string | null;
};

export type OverviewViewModel = {
  serviceState: ServiceState;
  startupEnabled: boolean | null;
  version: string | null;
  hotkeys: OverviewHotkeyViewModel[];
  statistics: {
    todayTriggers: number | null;
    weekTriggers: number | null;
    monthTriggers: number | null;
    savedMinutesThisMonth: number | null;
  };
  sourceAvailable: boolean;
};

export function getToolWheelShortcutLabel(hotkeys: OverviewHotkeyViewModel[]) {
  const toolWheel = hotkeys.find((hotkey) => hotkey.id === "toolWheel");
  const binding = toolWheel?.binding?.trim();
  if (binding && toolWheel?.enabled === true && toolWheel.state === "normal") {
    return `${binding} 呼出`;
  }
  if (binding && toolWheel?.state === "conflict") {
    return `${binding} 冲突`;
  }
  return binding ? "快捷键尚未生效" : "快捷键服务未接入";
}

const EMPTY_STATISTICS: OverviewViewModel["statistics"] = {
  todayTriggers: null,
  weekTriggers: null,
  monthTriggers: null,
  savedMinutesThisMonth: null
};

export function createOverviewViewModel(
  snapshot: OverviewBackendSnapshot | null
): OverviewViewModel {
  const hotkeysById = snapshot?.hotkeys
    ? new Map(snapshot.hotkeys.map((hotkey) => [hotkey.id, hotkey]))
    : null;

  return {
    serviceState: snapshot?.serviceState ?? "unknown",
    startupEnabled: snapshot?.startupEnabled ?? null,
    version: snapshot?.version ?? null,
    hotkeys: GLOBAL_HOTKEY_DEFINITIONS.map((presentation) => {
      const runtime = hotkeysById?.get(presentation.id);

      return {
        id: presentation.id as OverviewHotkeyId,
        title: presentation.title,
        description: presentation.description,
        binding: runtime?.binding ?? null,
        enabled: runtime?.enabled ?? null,
        state: runtime?.state ?? "unavailable",
        detail: runtime?.detail ?? null
      };
    }),
    statistics: snapshot?.statistics ?? EMPTY_STATISTICS,
    sourceAvailable: snapshot !== null
  };
}

export const EMPTY_OVERVIEW_VIEW_MODEL = createOverviewViewModel(null);
