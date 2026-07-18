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
  iconTone: "note" | "chrome" | "image" | "excel" | "word" | "snipaste";
};

export type ClipboardSettingsViewModel = {
  retentionDays: string;
  maxItems: string;
  ignoredApps: string;
  duplicateStrategy: string;
  sensitiveRules: string;
};

export type ClipboardActionAvailability = {
  canCopy: boolean;
  canTypeIntoTarget: boolean;
  canFavorite: boolean;
  canDelete: boolean;
  canOpenSource: boolean;
  canClearHistory: boolean;
};

export type ClipboardPageViewModel = {
  monitoring: "paused" | "running" | "unavailable";
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
    retentionDays: "30 天",
    maxItems: "1000",
    ignoredApps: "",
    duplicateStrategy: "合并（保留最新）",
    sensitiveRules: ""
  },
  actions: {
    canCopy: false,
    canTypeIntoTarget: false,
    canFavorite: true,
    canDelete: false,
    canOpenSource: false,
    canClearHistory: false
  }
};
