export const THEME_MODES = ["system", "light", "dark"] as const;
export const THEME_ACCENTS = [
  "#216bd9",
  "#7955c7",
  "#008b83",
  "#c7427a",
  "#e36a00",
  "#6d7782"
] as const;
export const ANIMATION_SPEEDS = ["slow", "normal", "fast"] as const;

export type ThemeMode = (typeof THEME_MODES)[number];
export type ThemeAccent = (typeof THEME_ACCENTS)[number];
export type AnimationSpeed = (typeof ANIMATION_SPEEDS)[number];

export type ThemePreferences = {
  mode: ThemeMode;
  accent: ThemeAccent;
  animationSpeed: AnimationSpeed;
  reduceTransparency: boolean;
};

export type ThemeSnapshot = ThemePreferences & {
  revision: number;
};

export type ThemePatch = Partial<ThemePreferences>;

export type ThemeBroadcastWarning = {
  code: string;
  message: string;
};

export type ThemeUpdateResult = {
  snapshot: ThemeSnapshot;
  broadcastWarning: ThemeBroadcastWarning | null;
};

export type ThemeCommandError = {
  code: string;
  message: string;
  field: string | null;
  retryable: boolean;
  applied: boolean;
};

export type ThemeControllerStatus = "loading" | "ready" | "unavailable";

export type ThemeControllerState = {
  status: ThemeControllerStatus;
  confirmed: ThemeSnapshot | null;
  current: ThemeSnapshot | null;
  saving: boolean;
  error: ThemeCommandError | null;
  warning: ThemeBroadcastWarning | null;
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function isOneOf<T extends readonly string[]>(value: unknown, values: T): value is T[number] {
  return typeof value === "string" && values.includes(value as T[number]);
}

function requiredString(record: Record<string, unknown>, key: string): string {
  const value = record[key];
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`Invalid theme payload field: ${key}`);
  }
  return value;
}

export function parseThemeSnapshot(value: unknown): ThemeSnapshot {
  if (!isRecord(value)) {
    throw new Error("Invalid theme snapshot payload");
  }

  const { mode, accent, animationSpeed, reduceTransparency, revision } = value;
  if (!isOneOf(mode, THEME_MODES)) {
    throw new Error("Invalid theme payload field: mode");
  }
  if (!isOneOf(accent, THEME_ACCENTS)) {
    throw new Error("Invalid theme payload field: accent");
  }
  if (!isOneOf(animationSpeed, ANIMATION_SPEEDS)) {
    throw new Error("Invalid theme payload field: animationSpeed");
  }
  if (typeof reduceTransparency !== "boolean") {
    throw new Error("Invalid theme payload field: reduceTransparency");
  }
  if (!Number.isSafeInteger(revision) || Number(revision) < 0) {
    throw new Error("Invalid theme payload field: revision");
  }

  return {
    mode,
    accent,
    animationSpeed,
    reduceTransparency,
    revision: Number(revision)
  };
}

export function parseThemeUpdateResult(value: unknown): ThemeUpdateResult {
  if (!isRecord(value)) {
    throw new Error("Invalid theme update payload");
  }

  const broadcastWarningValue = value.broadcastWarning;
  let broadcastWarning: ThemeBroadcastWarning | null = null;
  if (broadcastWarningValue !== null && broadcastWarningValue !== undefined) {
    if (!isRecord(broadcastWarningValue)) {
      throw new Error("Invalid theme payload field: broadcastWarning");
    }
    broadcastWarning = {
      code: requiredString(broadcastWarningValue, "code"),
      message: requiredString(broadcastWarningValue, "message")
    };
  }

  return {
    snapshot: parseThemeSnapshot(value),
    broadcastWarning
  };
}

export function normalizeThemeCommandError(value: unknown): ThemeCommandError {
  if (isRecord(value)) {
    return {
      code: typeof value.code === "string" ? value.code : "theme_client_failed",
      message: typeof value.message === "string" ? value.message : "主题设置操作失败。",
      field: typeof value.field === "string" ? value.field : null,
      retryable: typeof value.retryable === "boolean" ? value.retryable : false,
      applied: typeof value.applied === "boolean" ? value.applied : false
    };
  }

  return {
    code: "theme_client_failed",
    message: value instanceof Error ? value.message : "主题设置操作失败。",
    field: null,
    retryable: false,
    applied: false
  };
}

export function applyThemePatch(snapshot: ThemeSnapshot, patch: ThemePatch): ThemeSnapshot {
  return { ...snapshot, ...patch, revision: snapshot.revision };
}
