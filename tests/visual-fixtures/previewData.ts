import clipboardPreviewLandscape from "../../src/assets/clipboard-preview-landscape.svg";
import clipboardPreviewWindow from "../../src/assets/clipboard-preview-window.svg";
import type { ClipboardPageViewModel } from "../../src/app/clipboardModel";
import type { OverviewBackendSnapshot } from "../../src/app/overviewModel";

// This module is intentionally outside src/. Visual harnesses and tests may
// import it; production application modules must never depend on it.
export const OVERVIEW_PREVIEW_DATA: OverviewBackendSnapshot = {
  version: "1.3.0",
  serviceState: "running",
  startupEnabled: true,
  hotkeys: [
    { id: "capture", binding: "F1", enabled: true, state: "normal", detail: null },
    { id: "clipboardQr", binding: "F4", enabled: true, state: "normal", detail: null },
    { id: "toolWheel", binding: "Alt+Space", enabled: true, state: "normal", detail: null },
    {
      id: "clipboardPanel",
      binding: "Win+V",
      enabled: true,
      state: "conflict",
      detail: "与系统快捷键冲突"
    }
  ],
  statistics: {
    todayTriggers: 128,
    weekTriggers: 1248,
    monthTriggers: 8653,
    savedMinutesThisMonth: 9120
  }
};

export const CLIPBOARD_PREVIEW_DATA: ClipboardPageViewModel = {
  monitoring: "paused",
  totalCount: 28,
  items: [
    {
      id: "text-open-desk-tools",
      kind: "text",
      title: "OpenDeskTools 是一款高效的桌面增强工具，帮助你更好地管理剪贴板、快速启动应用...",
      preview:
        "OpenDeskTools 是一款高效的桌面增强工具，帮助你更好地管理剪贴板、快速启动应用、截图、生成二维码等，让你的工作效率倍增。",
      sourceApp: "记事本",
      sourceProcess: "notepad.exe",
      capturedAt: "2024-05-21 10:24:31",
      time: "10:24:31",
      size: "118 字符（236 字节）",
      favorite: false,
      locked: true,
      privacy: "sensitive",
      iconTone: "note"
    },
    {
      id: "text-url",
      kind: "text",
      title: "https://open-desktools.github.io",
      preview: "https://open-desktools.github.io",
      sourceApp: "Google Chrome",
      sourceProcess: "chrome.exe",
      capturedAt: "2024-05-21 10:23:18",
      time: "10:23:18",
      size: "34 字符（34 字节）",
      favorite: false,
      locked: false,
      privacy: "normal",
      iconTone: "chrome"
    },
    {
      id: "image-landscape",
      kind: "image",
      title: "2024-05-21_10-22-45.png",
      preview: "图片预览：湖泊、山脉与天空截图，尺寸 960 × 540。",
      previewImageUrl: clipboardPreviewLandscape,
      sourceApp: "截图工具",
      sourceProcess: "OpenDeskTools.exe",
      capturedAt: "2024-05-21 10:22:45",
      time: "10:22:45",
      size: "960 × 540，420 KB",
      favorite: false,
      locked: true,
      privacy: "unknown",
      iconTone: "image"
    },
    {
      id: "text-excel",
      kind: "text",
      title: "销售数据汇总表\n项目,金额,日期,负责人\n产品A,12000,2024-05-20,张三\n产品B,9800,2024-05-20,李四...",
      preview:
        "销售数据汇总表\n项目,金额,日期,负责人\n产品A,12000,2024-05-20,张三\n产品B,9800,2024-05-20,李四",
      sourceApp: "Microsoft Excel",
      sourceProcess: "EXCEL.EXE",
      capturedAt: "2024-05-21 10:21:07",
      time: "10:21:07",
      size: "86 字符（172 字节）",
      favorite: false,
      locked: false,
      privacy: "normal",
      iconTone: "excel"
    },
    {
      id: "text-word",
      kind: "text",
      title: "会议纪要：\n1. 确认需求文档\n2. 设计原型评审",
      preview: "会议纪要：\n1. 确认需求文档\n2. 设计原型评审",
      sourceApp: "Microsoft Word",
      sourceProcess: "WINWORD.EXE",
      capturedAt: "2024-05-21 10:20:12",
      time: "10:20:12",
      size: "31 字符（62 字节）",
      favorite: true,
      locked: true,
      privacy: "unknown",
      iconTone: "word"
    },
    {
      id: "image-snipaste",
      kind: "image",
      title: "Snipaste_2024-05-21_10-19-33.png",
      preview: "图片预览：白色窗口局部截图，尺寸 512 × 256。",
      previewImageUrl: clipboardPreviewWindow,
      sourceApp: "Snipaste",
      sourceProcess: "Snipaste.exe",
      capturedAt: "2024-05-21 10:19:33",
      time: "10:19:33",
      size: "512 × 256，96 KB",
      favorite: false,
      locked: false,
      privacy: "normal",
      iconTone: "image"
    }
  ],
  settings: {
    retentionDays: "30 天",
    maxItems: "1000",
    ignoredApps: "password.exe, 1password.exe, bitwarden.exe",
    duplicateStrategy: "合并（保留最新）",
    sensitiveRules: "password\npasswd\n密钥\nsecret\n\\b[A-Za-z0-9+/]{20,}={0,2}"
  },
  actions: {
    canCopy: false,
    canTypeIntoTarget: false,
    canFavorite: true,
    canDelete: true,
    canOpenSource: false,
    canClearHistory: false
  }
};
