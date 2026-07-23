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
export const BACKGROUND_FITS = ["cover", "contain"] as const;

export type ThemeMode = (typeof THEME_MODES)[number];
export type ThemeAccent = string;
export type AnimationSpeed = (typeof ANIMATION_SPEEDS)[number];
export type BackgroundFit = (typeof BACKGROUND_FITS)[number];

export type ThemeBackgroundAsset = {
  id: string;
  fileName: string;
  byteSize: number;
  width: number;
  height: number;
};

export type ThemePreferences = {
  mode: ThemeMode;
  accent: ThemeAccent;
  animationSpeed: AnimationSpeed;
  reduceTransparency: boolean;
  background: ThemeBackgroundAsset | null;
  backgroundFit: BackgroundFit;
  backgroundDim: number;
  backgroundBlur: number;
  panelOpacity: number;
};

export type ThemeSnapshot = ThemePreferences & {
  revision: number;
};

export type ThemePatch = Partial<Omit<ThemePreferences, "background">>;

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

function requiredSafeInteger(
  record: Record<string, unknown>,
  key: string,
  minimum: number,
  maximum: number
): number {
  const value = record[key];
  if (!Number.isSafeInteger(value) || Number(value) < minimum || Number(value) > maximum) {
    throw new Error(`Invalid theme payload field: ${key}`);
  }
  return Number(value);
}

function parseThemeBackgroundAsset(value: unknown): ThemeBackgroundAsset | null {
  if (value === null) {
    return null;
  }
  if (!isRecord(value)) {
    throw new Error("Invalid theme payload field: background");
  }
  const id = requiredString(value, "id");
  const fileName = requiredString(value, "fileName");
  if (!/^[a-f0-9]{64}$/.test(id) || fileName.includes("/") || fileName.includes("\\")) {
    throw new Error("Invalid theme payload field: background");
  }
  return {
    id,
    fileName,
    byteSize: requiredSafeInteger(value, "byteSize", 1, 64 * 1024 * 1024),
    width: requiredSafeInteger(value, "width", 1, 16_384),
    height: requiredSafeInteger(value, "height", 1, 16_384)
  };
}

export function parseThemeSnapshot(value: unknown): ThemeSnapshot {
  if (!isRecord(value)) {
    throw new Error("Invalid theme snapshot payload");
  }

  const {
    mode,
    accent,
    animationSpeed,
    reduceTransparency,
    background,
    backgroundFit,
    backgroundDim,
    backgroundBlur,
    panelOpacity,
    revision
  } = value;
  if (!isOneOf(mode, THEME_MODES)) {
    throw new Error("Invalid theme payload field: mode");
  }
  if (typeof accent !== "string" || !/^#[0-9a-f]{6}$/.test(accent)) {
    throw new Error("Invalid theme payload field: accent");
  }
  if (!isOneOf(animationSpeed, ANIMATION_SPEEDS)) {
    throw new Error("Invalid theme payload field: animationSpeed");
  }
  if (typeof reduceTransparency !== "boolean") {
    throw new Error("Invalid theme payload field: reduceTransparency");
  }
  if (!isOneOf(backgroundFit, BACKGROUND_FITS)) {
    throw new Error("Invalid theme payload field: backgroundFit");
  }
  if (!Number.isSafeInteger(revision) || Number(revision) < 0) {
    throw new Error("Invalid theme payload field: revision");
  }

  return {
    mode,
    accent,
    animationSpeed,
    reduceTransparency,
    background: parseThemeBackgroundAsset(background),
    backgroundFit,
    backgroundDim: requiredSafeInteger(value, "backgroundDim", 0, 100),
    backgroundBlur: requiredSafeInteger(value, "backgroundBlur", 0, 24),
    panelOpacity: requiredSafeInteger(value, "panelOpacity", 0, 100),
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
