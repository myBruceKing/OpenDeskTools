export type ClipboardFilter = "all" | "text" | "image" | "favorite";

export type ClipboardItemKind = "text" | "image";

export type ClipboardPrivacy = "normal" | "sensitive" | "unknown";

export type ClipboardItemViewModel = {
  id: string;
  kind: ClipboardItemKind;
  title: string;
  preview: string;
  previewImageUrl?: string;
  sourceApp: string;
  sourceProcess: string;
  capturedAt: string;
  time: string;
  size: string;
  favorite: boolean;
  locked: boolean;
  privacy: ClipboardPrivacy;
  iconTone: "note" | "chrome" | "image" | "excel" | "word";
};

export type ClipboardSettingsViewModel = {
  retentionDays: string | null;
  maxItems: string | null;
  ignoredApps: string | null;
  duplicateStrategy: string | null;
  sensitiveRules: string | null;
};

export type ClipboardMonitoringState = "paused" | "running" | "unavailable";

export function getClipboardMonitoringPresentation(monitoring: ClipboardMonitoringState) {
  switch (monitoring) {
    case "running":
      return { label: "监控运行中", checked: true, disabled: true } as const;
    case "paused":
      return { label: "监控已暂停", checked: false, disabled: true } as const;
    case "unavailable":
    default:
      return { label: "监控不可用", checked: null, disabled: true } as const;
  }
}

export type ClipboardActionAvailability = {
  canCopy: boolean;
  canTypeIntoTarget: boolean;
  canFavorite: boolean;
  canDelete: boolean;
  canOpenSource: boolean;
  canClearHistory: boolean;
};

export type ClipboardPageViewModel = {
  monitoring: ClipboardMonitoringState;
  totalCount: number;
  items: ClipboardItemViewModel[];
  settings: ClipboardSettingsViewModel;
  actions: ClipboardActionAvailability;
};

export const EMPTY_CLIPBOARD_VIEW_MODEL: ClipboardPageViewModel = {
  monitoring: "unavailable",
  totalCount: 0,
  items: [],
  settings: {
    retentionDays: null,
    maxItems: null,
    ignoredApps: null,
    duplicateStrategy: null,
    sensitiveRules: null
  },
  actions: {
    canCopy: false,
    canTypeIntoTarget: false,
    canFavorite: false,
    canDelete: false,
    canOpenSource: false,
    canClearHistory: false
  }
};
