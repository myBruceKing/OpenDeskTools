import { invoke } from "@tauri-apps/api/core";

export type QuickLaunchAppPayload = {
  id: string;
  name: string;
  path: string;
  arguments: string;
  workingDirectory: string | null;
  iconPath: string;
  iconIndex: number;
  source: string;
  visible: boolean;
  available: boolean;
  iconAvailable: boolean;
};

export type QuickLaunchSnapshotPayload = {
  pinnedApps: QuickLaunchAppPayload[];
  discoveredApps: QuickLaunchAppPayload[];
  toolMenu: ToolMenuPreferences;
};

export type ToolMenuLayout = "wheel" | "dock" | "vertical";
export type ToolMenuPreferences = { layout: ToolMenuLayout; keepOpenOnKeyRelease: boolean };

type InvokeFunction = (command: string, args?: Record<string, unknown>) => Promise<unknown>;

function parseApp(value: unknown): QuickLaunchAppPayload {
  if (!value || typeof value !== "object") throw new Error("快速启动数据无效");
  const app = value as Record<string, unknown>;
  if (
    typeof app.id !== "string" || typeof app.name !== "string" || typeof app.path !== "string" ||
    typeof app.arguments !== "string" || (app.workingDirectory !== null && typeof app.workingDirectory !== "string") ||
    typeof app.iconPath !== "string" || !Number.isInteger(app.iconIndex) ||
    typeof app.source !== "string" || typeof app.visible !== "boolean" ||
    typeof app.available !== "boolean" || typeof app.iconAvailable !== "boolean"
  ) throw new Error("快速启动数据无效");
  return app as unknown as QuickLaunchAppPayload;
}

function parseSnapshot(value: unknown): QuickLaunchSnapshotPayload {
  if (!value || typeof value !== "object") throw new Error("快速启动数据无效");
  const payload = value as Record<string, unknown>;
  const menu = payload.toolMenu;
  if (!Array.isArray(payload.pinnedApps) || !Array.isArray(payload.discoveredApps)
    || !menu || typeof menu !== "object"
    || !["wheel", "dock", "vertical"].includes((menu as Record<string, unknown>).layout as string)
    || typeof (menu as Record<string, unknown>).keepOpenOnKeyRelease !== "boolean") throw new Error("快速启动数据无效");
  return { pinnedApps: payload.pinnedApps.map(parseApp), discoveredApps: payload.discoveredApps.map(parseApp), toolMenu: menu as ToolMenuPreferences };
}

function binaryBlob(value: unknown) {
  if (value instanceof ArrayBuffer) {
    return new Blob([value], { type: "image/png" });
  }
  if (value instanceof Uint8Array) {
    const copy = new Uint8Array(value.byteLength);
    copy.set(value);
    return new Blob([copy.buffer], { type: "image/png" });
  }
  if (Array.isArray(value) && value.every((entry) => Number.isInteger(entry) && entry >= 0 && entry <= 255)) {
    return new Blob([new Uint8Array(value)], { type: "image/png" });
  }
  throw new Error("快速启动图标无效");
}

export function createQuickLaunchClient(invokeFunction: InvokeFunction = invoke) {
  const snapshot = (command: string, args?: Record<string, unknown>) =>
    invokeFunction(command, args).then(parseSnapshot);
  return {
    getSnapshot: () => snapshot("get_quick_launch_snapshot"),
    rescan: () => snapshot("rescan_quick_launch"),
    pin: (path: string, source?: string) => snapshot("pin_quick_launch_app", { input: { path, source } }),
    unpin: (path: string) => snapshot("unpin_quick_launch_app", { input: { path } }),
    setVisible: (path: string, visible: boolean) => snapshot("set_quick_launch_visible", { input: { path, visible } }),
    reorder: (activePath: string, overPath: string) => snapshot("reorder_quick_launch_apps", { input: { activePath, overPath } }),
    updateToolMenu: (preferences: ToolMenuPreferences) => snapshot("update_tool_menu_preferences", { input: { preferences } }),
    launch: (path: string) => invokeFunction("launch_quick_launch_app", { input: { path } }).then(() => undefined),
    selectFile: () => invokeFunction("select_quick_launch_app").then((value) => value === null || typeof value === "string" ? value : Promise.reject(new Error("选择结果无效"))),
    getIcon: (path: string) => invokeFunction("get_quick_launch_icon", { input: { path } }).then(binaryBlob)
  };
}

export const quickLaunchClient = createQuickLaunchClient();
