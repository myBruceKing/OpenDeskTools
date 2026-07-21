export type ClipboardFilter = "all" | "text" | "image" | "favorite";

export type ClipboardItemKind = "text" | "image" | "files";
export type ClipboardDisplayCategory = "text" | "image" | "files";

export type ClipboardPrivacy = "normal" | "sensitive" | "unknown";

export type ClipboardItemViewModel = {
  id: string;
  revision: number;
  kind: ClipboardItemKind;
  title: string;
  preview: string;
  sourceApp: string;
  sourceProcess: string;
  capturedAt: string;
  time: string;
  size: string;
  favorite: boolean;
  locked: boolean;
  privacy: ClipboardPrivacy;
  sourceIconAvailable: boolean;
  iconTone: "note" | "chrome" | "image" | "excel" | "word";
  displayCategory?: ClipboardDisplayCategory;
};

export type ClipboardSettingsViewModel = {
  retentionDays: string | null;
  maxItems: string | null;
  ignoredApps: string | null;
  duplicateStrategy: string | null;
  sensitiveRules: string | null;
};

export type ClipboardMonitoringState = "paused" | "running" | "unavailable";

export type ClipboardHistoryQuery = {
  scope: "all" | "favorites";
  search?: string | null;
  limit?: number | null;
};

export type ClipboardHistoryItem = {
  id: string;
  revision: number;
  kind: ClipboardItemKind;
  textContent: string | null;
  sourceApplication: string | null;
  sourceProcess: string | null;
  capturedAtMs: number;
  byteSize: number;
  isFavorite: boolean;
  sourceIconAvailable: boolean;
  fileCount?: number | null;
  fileNames?: string[] | null;
  displayCategory?: ClipboardDisplayCategory;
};

export type ClipboardHistoryResult = {
  items: ClipboardHistoryItem[];
  totalCount: number;
  monitoring: "running" | "unavailable";
  surfaceActive: boolean;
  inputAvailable: boolean;
};

export type ClipboardCommandError = {
  code: string;
  message: string;
  retryable: boolean;
};

export type ClipboardItemAction = "copy" | "input";

export type ClipboardItemActionResult = {
  action: "copied" | "input";
  clipboardUpdated: true;
};

export type ClipboardSurfaceCloseResult = {
  closed: true;
  inputAvailable: false;
};

export type ClipboardItemActionState = {
  action: ClipboardItemAction;
  itemId: string;
  status: "pending" | "success" | "error";
  message: string;
  code: string | null;
  retryable: boolean;
};

export type ClipboardTextEditState = {
  itemId: string;
  status: "pending" | "success" | "error";
  message: string;
  code: string | null;
  retryable: boolean;
};

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
  canEditText: boolean;
};

export type ClipboardPageViewModel = {
  monitoring: ClipboardMonitoringState;
  totalCount: number;
  items: ClipboardItemViewModel[];
  settings: ClipboardSettingsViewModel;
  actions: ClipboardActionAvailability;
};

export type ClipboardControllerStatus = "loading" | "ready" | "unavailable";

export type ClipboardControllerState = {
  status: ClipboardControllerStatus;
  viewModel: ClipboardPageViewModel;
  error: ClipboardCommandError | null;
  realtimeError: ClipboardCommandError | null;
  pendingItemIds: readonly string[];
  itemAction: ClipboardItemActionState | null;
  textEdit: ClipboardTextEditState | null;
  surfaceActive: boolean;
  surfaceClosing: boolean;
  surfaceError: ClipboardCommandError | null;
  clearing: boolean;
};

const READY_SETTINGS: ClipboardSettingsViewModel = {
  retentionDays: null,
  maxItems: "100",
  ignoredApps: null,
  duplicateStrategy: "相同内容移到最前",
  sensitiveRules: null
};

const DISABLED_ACTIONS: ClipboardActionAvailability = {
  canCopy: false,
  canTypeIntoTarget: false,
  canFavorite: false,
  canDelete: false,
  canOpenSource: false,
  canClearHistory: false,
  canEditText: false
};

const READY_ACTIONS: ClipboardActionAvailability = {
  ...DISABLED_ACTIONS,
  canCopy: true,
  canFavorite: true,
  canDelete: true,
  canClearHistory: true,
  canEditText: true
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
  actions: DISABLED_ACTIONS
};

export function createClipboardLoadingState(surfaceActive = false): ClipboardControllerState {
  return {
    status: "loading",
    viewModel: {
      ...EMPTY_CLIPBOARD_VIEW_MODEL,
      monitoring: "paused",
      settings: READY_SETTINGS
    },
    error: null,
    realtimeError: null,
    pendingItemIds: [],
    itemAction: null,
    textEdit: null,
    surfaceActive,
    surfaceClosing: false,
    surfaceError: null,
    clearing: false
  };
}

function truncateText(value: string, maximumLength: number) {
  const characters = Array.from(value);
  return characters.length <= maximumLength
    ? value
    : `${characters.slice(0, maximumLength).join("")}…`;
}

function formatBytes(byteSize: number) {
  if (byteSize < 1024) {
    return `${byteSize} B`;
  }
  if (byteSize < 1024 * 1024) {
    return `${(byteSize / 1024).toFixed(byteSize < 10 * 1024 ? 1 : 0)} KB`;
  }
  return `${(byteSize / (1024 * 1024)).toFixed(byteSize < 10 * 1024 * 1024 ? 1 : 0)} MB`;
}

function formatDateParts(capturedAtMs: number) {
  const date = new Date(capturedAtMs);
  if (Number.isNaN(date.getTime())) {
    return { capturedAt: "时间不可用", time: "—" };
  }
  const pad = (value: number) => String(value).padStart(2, "0");
  const time = `${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
  return {
    capturedAt: `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${time}`,
    time
  };
}

export function toClipboardItemViewModel(item: ClipboardHistoryItem): ClipboardItemViewModel {
  const sourceApp = item.sourceApplication?.trim() || "来源不可用";
  const sourceProcess = item.sourceProcess?.trim() || "来源不可用";
  const date = formatDateParts(item.capturedAtMs);
  const displayCategory = item.displayCategory ?? item.kind;

  if (item.kind === "image") {
    return {
      id: item.id,
      revision: item.revision,
      kind: "image",
      title: "图片内容",
      preview: "图片预览暂不可用",
      sourceApp,
      sourceProcess,
      ...date,
      size: formatBytes(item.byteSize),
      favorite: item.isFavorite,
      locked: false,
      privacy: "unknown",
      sourceIconAvailable: item.sourceIconAvailable,
      iconTone: "image",
      displayCategory
    };
  }

  if (item.kind === "files") {
    const names = item.fileNames ?? [];
    const count = item.fileCount ?? names.length;
    return {
      id: item.id,
      revision: item.revision,
      kind: "files",
      title: count === 1 ? (names[0] ?? "文件") : `${count} 个文件`,
      preview: names.join("\n"),
      sourceApp,
      sourceProcess,
      ...date,
      size: formatBytes(item.byteSize),
      favorite: item.isFavorite,
      locked: false,
      privacy: "unknown",
      sourceIconAvailable: item.sourceIconAvailable,
      iconTone: displayCategory === "image" ? "image" : "note",
      displayCategory
    };
  }

  const rawText = item.textContent ?? "";
  const firstReadableLine = rawText
    .split(/\r?\n/)
    .map((line) => line.trim())
    .find(Boolean);
  const title = truncateText(firstReadableLine ?? "空文本", 72);
  const preview = rawText.length > 0 ? rawText : "（空文本）";

  return {
    id: item.id,
    revision: item.revision,
    kind: "text",
    title,
    preview,
    sourceApp,
    sourceProcess,
    ...date,
    size: `${Array.from(rawText).length} 字符（${formatBytes(item.byteSize)}）`,
    favorite: item.isFavorite,
    locked: false,
    privacy: "unknown",
    sourceIconAvailable: item.sourceIconAvailable,
    iconTone: "note",
    displayCategory
  };
}

export function createClipboardReadyViewModel(result: ClipboardHistoryResult): ClipboardPageViewModel {
  return {
    monitoring: result.monitoring,
    totalCount: result.totalCount,
    items: result.items.map(toClipboardItemViewModel),
    settings: READY_SETTINGS,
    actions: {
      ...READY_ACTIONS,
      canTypeIntoTarget: result.inputAvailable
    }
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function requiredString(record: Record<string, unknown>, key: string) {
  const value = record[key];
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`Invalid clipboard payload field: ${key}`);
  }
  return value;
}

function canonicalPositiveI64(record: Record<string, unknown>, key: string) {
  const value = requiredString(record, key);
  if (!/^[1-9]\d*$/.test(value) || BigInt(value) > 9_223_372_036_854_775_807n) {
    throw new Error(`Invalid clipboard payload field: ${key}`);
  }
  return value;
}

function nullableString(record: Record<string, unknown>, key: string) {
  const value = record[key];
  if (value !== null && typeof value !== "string") {
    throw new Error(`Invalid clipboard payload field: ${key}`);
  }
  return value as string | null;
}

function nonNegativeInteger(record: Record<string, unknown>, key: string) {
  const value = record[key];
  if (!Number.isSafeInteger(value) || Number(value) < 0) {
    throw new Error(`Invalid clipboard payload field: ${key}`);
  }
  return Number(value);
}

export function parseClipboardHistoryItem(value: unknown): ClipboardHistoryItem {
  if (!isRecord(value)) {
    throw new Error("Invalid clipboard item payload");
  }
  const kind = value.kind;
  if (kind !== "text" && kind !== "image" && kind !== "files") {
    throw new Error("Invalid clipboard payload field: kind");
  }
  const textContent = nullableString(value, "textContent");
  if (
    (kind === "text" && textContent === null)
    || ((kind === "image" || kind === "files") && textContent !== null)
  ) {
    throw new Error("Invalid clipboard payload field: textContent");
  }
  if (typeof value.isFavorite !== "boolean") {
    throw new Error("Invalid clipboard payload field: isFavorite");
  }
  const rawFileCount = value.fileCount;
  const rawFileNames = value.fileNames;
  const fileCount = rawFileCount === undefined || rawFileCount === null
    ? null
    : nonNegativeInteger(value, "fileCount");
  const fileNames = rawFileNames === undefined || rawFileNames === null
    ? null
    : (() => {
      if (!Array.isArray(rawFileNames) || rawFileNames.some((name) => (
        typeof name !== "string" || name.length === 0 || /[\\/]/.test(name)
      ))) {
        throw new Error("Invalid clipboard payload field: fileNames");
      }
      return [...rawFileNames];
    })();
  if (kind === "files") {
    if (fileCount === null || fileCount <= 0 || fileNames === null || fileNames.length !== fileCount) {
      throw new Error("Invalid clipboard payload field: fileCount");
    }
  } else if (fileCount !== null || fileNames !== null) {
    throw new Error("Invalid clipboard payload field: fileNames");
  }
  const displayCategory = value.displayCategory ?? (kind === "files" ? null : kind);
  if (displayCategory !== "text" && displayCategory !== "image" && displayCategory !== "files") {
    throw new Error("Invalid clipboard payload field: displayCategory");
  }
  if (kind !== "files" && displayCategory !== kind) {
    throw new Error("Invalid clipboard payload field: displayCategory");
  }

  return {
    id: canonicalPositiveI64(value, "id"),
    revision: nonNegativeInteger(value, "revision"),
    kind,
    textContent,
    sourceApplication: nullableString(value, "sourceApplication"),
    sourceProcess: nullableString(value, "sourceProcess"),
    capturedAtMs: nonNegativeInteger(value, "capturedAtMs"),
    byteSize: nonNegativeInteger(value, "byteSize"),
    isFavorite: value.isFavorite,
    sourceIconAvailable: (() => {
      if (typeof value.sourceIconAvailable !== "boolean") {
        throw new Error("Invalid clipboard payload field: sourceIconAvailable");
      }
      return value.sourceIconAvailable;
    })(),
    fileCount,
    fileNames,
    displayCategory
  };
}

export function parseClipboardHistoryResult(value: unknown): ClipboardHistoryResult {
  if (!isRecord(value) || !Array.isArray(value.items)) {
    throw new Error("Invalid clipboard history payload");
  }
  const items = value.items.map(parseClipboardHistoryItem);
  const ids = new Set(items.map((item) => item.id));
  if (ids.size !== items.length) {
    throw new Error("Invalid clipboard history payload: duplicate id");
  }
  const totalCount = nonNegativeInteger(value, "totalCount");
  if (totalCount < items.length) {
    throw new Error("Invalid clipboard payload field: totalCount");
  }
  if (value.monitoring !== "running" && value.monitoring !== "unavailable") {
    throw new Error("Invalid clipboard payload field: monitoring");
  }
  if (typeof value.inputAvailable !== "boolean") {
    throw new Error("Invalid clipboard payload field: inputAvailable");
  }
  if (typeof value.surfaceActive !== "boolean") {
    throw new Error("Invalid clipboard payload field: surfaceActive");
  }
  return {
    items,
    totalCount,
    monitoring: value.monitoring,
    surfaceActive: value.surfaceActive,
    inputAvailable: value.inputAvailable
  };
}

export function parseClipboardDeleteResult(value: unknown): { deleted: boolean } {
  if (!isRecord(value) || typeof value.deleted !== "boolean") {
    throw new Error("Invalid clipboard delete payload");
  }
  return { deleted: value.deleted };
}

export function parseClipboardClearResult(value: unknown): { deletedCount: number } {
  if (!isRecord(value)) {
    throw new Error("Invalid clipboard clear payload");
  }
  return { deletedCount: nonNegativeInteger(value, "deletedCount") };
}

export function parseClipboardItemActionResult(
  value: unknown,
  expectedAction: ClipboardItemActionResult["action"]
): ClipboardItemActionResult {
  if (
    !isRecord(value)
    || value.action !== expectedAction
    || value.clipboardUpdated !== true
  ) {
    throw new Error("Invalid clipboard item action payload");
  }
  return {
    action: expectedAction,
    clipboardUpdated: true
  };
}

export function parseClipboardSurfaceCloseResult(value: unknown): ClipboardSurfaceCloseResult {
  if (
    !isRecord(value)
    || value.closed !== true
    || value.inputAvailable !== false
  ) {
    throw new Error("Invalid clipboard surface close payload");
  }
  return { closed: true, inputAvailable: false };
}

export function normalizeClipboardCommandError(value: unknown): ClipboardCommandError {
  const code = isRecord(value) && typeof value.code === "string"
    ? value.code
    : "clipboard_client_failed";
  const known: Record<string, Omit<ClipboardCommandError, "code">> = {
    invalid_clipboard_item_id: {
      message: "剪贴板记录标识无效。",
      retryable: false
    },
    clipboard_content_unavailable: {
      message: "当前剪贴板内容类型暂不支持。",
      retryable: false
    },
    invalid_clipboard_history_query: {
      message: "剪贴板历史查询条件无效。",
      retryable: false
    },
    clipboard_item_not_found: {
      message: "这条剪贴板记录已不存在，请重新打开页面。",
      retryable: false
    },
    invalid_clipboard_content: {
      message: "剪贴板内容无效，无法完成操作。",
      retryable: false
    },
    clipboard_history_unavailable: {
      message: "剪贴板历史服务暂时不可用，请稍后重试。",
      retryable: true
    },
    clipboard_operation_not_applied: {
      message: "剪贴板操作未完成，请刷新后重试。",
      retryable: true
    },
    clipboard_write_unavailable: {
      message: "无法写入系统剪贴板，请重试。",
      retryable: true
    },
    clipboard_write_failed: {
      message: "Windows 未完成剪贴板写入，请重试该记录。",
      retryable: true
    },
    clipboard_target_unavailable: {
      message: "目标窗口已不可用，请重新选择目标。",
      retryable: false
    },
    clipboard_target_focus_denied: {
      message: "暂时无法聚焦目标窗口，请重试。",
      retryable: true
    },
    clipboard_input_denied: {
      message: "权限级别不一致，无法输入到目标窗口。",
      retryable: false
    },
    clipboard_input_cleanup_failed: {
      message: "按键释放未确认，输入已暂停。",
      retryable: false
    },
    clipboard_subscription_unavailable: {
      message: "剪贴板实时更新暂时不可用，当前历史仍可查看。",
      retryable: true
    },
    clipboard_edit_empty: {
      message: "内容不能为空。",
      retryable: false
    },
    clipboard_edit_duplicate: {
      message: "已存在相同内容，未保存。",
      retryable: false
    },
    clipboard_revision_conflict: {
      message: "内容已在其他位置更新，请重新编辑。",
      retryable: true
    },
    clipboard_source_icon_unavailable: {
      message: "来源图标暂不可用。",
      retryable: false
    },
    clipboard_files_unavailable: {
      message: "一个或多个原文件已不存在，无法复制或输入。",
      retryable: false
    }
  };
  const presentation = known[code];
  if (presentation) {
    return { code, ...presentation };
  }
  return {
    code: "clipboard_client_failed",
    message: "剪贴板历史服务暂时不可用，请稍后重试。",
    retryable: true
  };
}
