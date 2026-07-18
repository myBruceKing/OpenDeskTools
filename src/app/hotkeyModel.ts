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
