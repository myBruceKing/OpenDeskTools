export type HotkeyState = "normal" | "conflict" | "unavailable" | "unknown";

export type GlobalHotkeyId =
  | "capture"
  | "pinImage"
  | "clipboardQr"
  | "toolWheel"
  | "clipboardPanel";

export type GlobalHotkeyDefinition = {
  id: GlobalHotkeyId;
  title: string;
  description: string;
  defaultBinding: string;
  showInOverview: boolean;
};

export const GLOBAL_HOTKEY_DEFINITIONS: ReadonlyArray<GlobalHotkeyDefinition> = [
  {
    id: "capture",
    title: "截图",
    description: "截图并进入编辑",
    defaultBinding: "F1",
    showInOverview: true
  },
  {
    id: "pinImage",
    title: "屏幕贴图",
    description: "将剪贴板图片贴到屏幕",
    defaultBinding: "F3",
    showInOverview: true
  },
  {
    id: "clipboardQr",
    title: "剪贴板二维码",
    description: "将剪贴板内容生成二维码",
    defaultBinding: "F4",
    showInOverview: true
  },
  {
    id: "toolWheel",
    title: "工具盘",
    description: "按住呼出圆盘菜单",
    defaultBinding: "Alt+Space",
    showInOverview: true
  },
  {
    id: "clipboardPanel",
    title: "剪贴板面板",
    description: "打开剪贴板历史面板",
    defaultBinding: "Win+V",
    showInOverview: true
  }
];

export const OVERVIEW_HOTKEY_DEFINITIONS = GLOBAL_HOTKEY_DEFINITIONS.filter(
  (definition) => definition.showInOverview
);

export const HOTKEY_CLASSIFICATIONS = [
  "ordinary",
  "system_reserved",
  "blocked",
  "unsupported_sequence"
] as const;

export type HotkeyClassificationKind = (typeof HOTKEY_CLASSIFICATIONS)[number];

export const HOTKEY_RUNTIME_STATES = [
  "registered",
  "conflict",
  "disabled",
  "unavailable",
  "degraded"
] as const;

export type HotkeyRuntimeState = (typeof HOTKEY_RUNTIME_STATES)[number];

export const HOTKEY_RUNTIME_BACKENDS = ["standard", "low_level_hook"] as const;

export type HotkeyRuntimeBackend = (typeof HOTKEY_RUNTIME_BACKENDS)[number];

export type HotkeyActionSnapshot = {
  actionId: GlobalHotkeyId;
  binding: string;
  configuredEnabled: boolean;
  classification: HotkeyClassificationKind;
  runtimeState: HotkeyRuntimeState;
  runtimeBackend: HotkeyRuntimeBackend | null;
  detail: string | null;
  actionAvailable: boolean;
  forceOverrideSystem: boolean;
};

export type HotkeySnapshot = {
  revision: number;
  actions: HotkeyActionSnapshot[];
};

export type HotkeyClassification = {
  binding: string;
  normalizedBinding: string;
  classification: HotkeyClassificationKind;
  message: string;
  forceOverrideAllowed: boolean;
};

export type SystemHotkeyNotice = {
  binding: string;
  letter: string;
  restartRequired: boolean;
};

export type HotkeyUpdateResult = {
  snapshot: HotkeySnapshot;
  systemHotkeyNotice: SystemHotkeyNotice | null;
};

export type HotkeyUpdatePatch = {
  actionId: GlobalHotkeyId;
  expectedRevision: number;
  binding: string;
  forceOverrideSystem: boolean;
};

export type HotkeyCommandError = {
  code: string;
  message: string;
  actualRevision: number | null;
};

export type HotkeyEditorState = {
  actionId: GlobalHotkeyId;
  actionAvailable: boolean;
  binding: string;
  inputDirty: boolean;
  classificationStatus: "loading" | "ready" | "error";
  classification: HotkeyClassification | null;
  forceOverrideSystem: boolean;
  saving: boolean;
  error: HotkeyCommandError | null;
};

export type HotkeyControllerState = {
  status: "loading" | "ready" | "unavailable";
  snapshot: HotkeySnapshot | null;
  editor: HotkeyEditorState | null;
  error: HotkeyCommandError | null;
  systemHotkeyNotice: SystemHotkeyNotice | null;
};

export type StableHotkeyActionId =
  | "screenshot.capture"
  | "clipboard.pin_image"
  | "clipboard.qr_convert"
  | "launcher.open"
  | "clipboard.open_panel";

const STABLE_ACTION_BY_UI_ID: Record<GlobalHotkeyId, StableHotkeyActionId> = {
  capture: "screenshot.capture",
  pinImage: "clipboard.pin_image",
  clipboardQr: "clipboard.qr_convert",
  toolWheel: "launcher.open",
  clipboardPanel: "clipboard.open_panel"
};

const UI_ACTION_BY_STABLE_ID = Object.fromEntries(
  Object.entries(STABLE_ACTION_BY_UI_ID).map(([uiId, stableId]) => [stableId, uiId])
) as Record<StableHotkeyActionId, GlobalHotkeyId>;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function isStableActionId(value: unknown): value is StableHotkeyActionId {
  return typeof value === "string" && value in UI_ACTION_BY_STABLE_ID;
}

export function toStableHotkeyActionId(actionId: GlobalHotkeyId): StableHotkeyActionId {
  return STABLE_ACTION_BY_UI_ID[actionId];
}

function isRuntimeState(value: unknown): value is HotkeyRuntimeState {
  return typeof value === "string" && HOTKEY_RUNTIME_STATES.includes(value as HotkeyRuntimeState);
}

function isRuntimeBackend(value: unknown): value is HotkeyRuntimeBackend {
  return (
    typeof value === "string" &&
    HOTKEY_RUNTIME_BACKENDS.includes(value as HotkeyRuntimeBackend)
  );
}

function isClassification(value: unknown): value is HotkeyClassificationKind {
  return typeof value === "string" && HOTKEY_CLASSIFICATIONS.includes(value as HotkeyClassificationKind);
}

function parseAction(value: unknown): HotkeyActionSnapshot {
  if (!isRecord(value)) {
    throw new Error("Invalid hotkey action payload");
  }
  if (!isStableActionId(value.actionId)) {
    throw new Error("Invalid hotkey payload field: actionId");
  }
  if (typeof value.binding !== "string") {
    throw new Error("Invalid hotkey payload field: binding");
  }
  if (typeof value.configuredEnabled !== "boolean") {
    throw new Error("Invalid hotkey payload field: configuredEnabled");
  }
  if (!isClassification(value.classification)) {
    throw new Error("Invalid hotkey payload field: classification");
  }
  if (!isRuntimeState(value.runtimeState)) {
    throw new Error("Invalid hotkey payload field: runtimeState");
  }
  if (value.runtimeBackend !== null && !isRuntimeBackend(value.runtimeBackend)) {
    throw new Error("Invalid hotkey payload field: runtimeBackend");
  }
  if (typeof value.detail !== "string" && value.detail !== null) {
    throw new Error("Invalid hotkey payload field: detail");
  }
  if (typeof value.actionAvailable !== "boolean") {
    throw new Error("Invalid hotkey payload field: actionAvailable");
  }
  if (typeof value.forceOverrideSystem !== "boolean") {
    throw new Error("Invalid hotkey payload field: forceOverrideSystem");
  }

  return {
    actionId: UI_ACTION_BY_STABLE_ID[value.actionId],
    binding: value.binding,
    configuredEnabled: value.configuredEnabled,
    classification: value.classification,
    runtimeState: value.runtimeState,
    runtimeBackend: value.runtimeBackend,
    detail: value.detail,
    actionAvailable: value.actionAvailable,
    forceOverrideSystem: value.forceOverrideSystem
  };
}

export function toHotkeyBadgeState(runtimeState: HotkeyRuntimeState): HotkeyState {
  if (runtimeState === "registered") {
    return "normal";
  }
  if (runtimeState === "conflict") {
    return "conflict";
  }
  return "unavailable";
}

export function parseHotkeySnapshot(value: unknown): HotkeySnapshot {
  if (!isRecord(value)) {
    throw new Error("Invalid hotkey snapshot payload");
  }
  if (!Number.isSafeInteger(value.revision) || Number(value.revision) < 0) {
    throw new Error("Invalid hotkey payload field: revision");
  }
  if (!Array.isArray(value.actions)) {
    throw new Error("Invalid hotkey payload field: actions");
  }

  return {
    revision: Number(value.revision),
    actions: value.actions.map(parseAction)
  };
}

export function parseSystemHotkeyNotice(value: unknown): SystemHotkeyNotice | null {
  if (value === null || value === undefined) {
    return null;
  }
  if (!isRecord(value)) {
    throw new Error("Invalid system hotkey notice payload");
  }
  if (typeof value.binding !== "string") {
    throw new Error("Invalid system hotkey notice field: binding");
  }
  if (typeof value.letter !== "string") {
    throw new Error("Invalid system hotkey notice field: letter");
  }
  if (typeof value.restartRequired !== "boolean") {
    throw new Error("Invalid system hotkey notice field: restartRequired");
  }
  return {
    binding: value.binding,
    letter: value.letter,
    restartRequired: value.restartRequired
  };
}

export function parseHotkeyUpdateResult(value: unknown): HotkeyUpdateResult {
  if (!isRecord(value)) {
    throw new Error("Invalid hotkey update payload");
  }
  return {
    snapshot: parseHotkeySnapshot(value.snapshot),
    systemHotkeyNotice: parseSystemHotkeyNotice(value.systemHotkeyNotice ?? null)
  };
}

export function parseHotkeyClassification(value: unknown): HotkeyClassification {
  if (!isRecord(value) || !isClassification(value.classification)) {
    throw new Error("Invalid hotkey payload field: classification");
  }
  if (typeof value.binding !== "string") {
    throw new Error("Invalid hotkey payload field: binding");
  }
  if (typeof value.normalizedBinding !== "string") {
    throw new Error("Invalid hotkey payload field: normalizedBinding");
  }
  if (typeof value.message !== "string") {
    throw new Error("Invalid hotkey payload field: message");
  }
  if (typeof value.forceOverrideAllowed !== "boolean") {
    throw new Error("Invalid hotkey payload field: forceOverrideAllowed");
  }
  return {
    binding: value.binding,
    normalizedBinding: value.normalizedBinding,
    classification: value.classification,
    message: value.message,
    forceOverrideAllowed: value.forceOverrideAllowed
  };
}

export function normalizeHotkeyCommandError(value: unknown): HotkeyCommandError {
  if (isRecord(value)) {
    return {
      code: typeof value.code === "string" ? value.code : "hotkey_client_failed",
      message: typeof value.message === "string" ? value.message : "快捷键操作失败。",
      actualRevision: Number.isSafeInteger(value.actualRevision)
        ? Number(value.actualRevision)
        : null
    };
  }

  return {
    code: "hotkey_client_failed",
    message: value instanceof Error ? value.message : "快捷键操作失败。",
    actualRevision: null
  };
}
