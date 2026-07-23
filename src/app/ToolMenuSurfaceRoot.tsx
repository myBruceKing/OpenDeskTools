import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import { ToolMenuPreview, type ToolMenuPreviewItem } from "../components/patterns/ToolMenuPreview";
import { quickLaunchClient, type QuickLaunchSnapshotPayload, type ToolMenuLayout } from "./quickLaunchClient";
import { toToolMenuPreviewItems, useQuickLaunchViewModel } from "./quickLaunchModel";
import { useWindowSurfaceRuntime } from "./useWindowSurfaceRuntime";
import styles from "./ToolMenuSurfaceRoot.module.css";

declare global {
  interface Window {
    __OPENDESK_TOOL_MENU_LAYOUT?: ToolMenuLayout;
  }
}

function nativeToolMenuLayout(): ToolMenuLayout {
  const layout = window.__OPENDESK_TOOL_MENU_LAYOUT;
  return layout === "dock" || layout === "vertical" || layout === "wheel" ? layout : "wheel";
}

export function ToolMenuSurfaceRoot() {
  const quickLaunch = useQuickLaunchViewModel();
  const [message, setMessage] = useState<string | null>(null);
  const [nativeLayout, setNativeLayout] = useState<ToolMenuLayout>(nativeToolMenuLayout);
  useWindowSurfaceRuntime();

  useEffect(() => {
    let active = true;
    const syncSnapshot = ({ payload }: { payload: QuickLaunchSnapshotPayload }) => {
      if (active) quickLaunch.actions.syncSnapshot(payload);
    };
    const subscriptions = [
      listen<QuickLaunchSnapshotPayload>("quick-launch://changed", syncSnapshot),
      listen<QuickLaunchSnapshotPayload>("tool-menu://snapshot", syncSnapshot)
    ];
    return () => {
      active = false;
      subscriptions.forEach((subscription) => {
        void subscription.then((dispose) => dispose());
      });
    };
  }, [quickLaunch.actions]);

  useEffect(() => {
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        void invoke("close_tool_menu_surface");
      }
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, []);

  useEffect(() => {
    const syncNativeLayout = () => setNativeLayout(nativeToolMenuLayout());
    window.addEventListener("opendesk-tool-menu-layout", syncNativeLayout);
    syncNativeLayout();
    return () => window.removeEventListener("opendesk-tool-menu-layout", syncNativeLayout);
  }, []);

  const launch = useCallback(async (item: ToolMenuPreviewItem) => {
    const app = quickLaunch.visiblePinnedApps.find((candidate) => (candidate.id ?? candidate.path) === item.id);
    if (!app || app.available === false) {
      setMessage("该固定程序当前不可用。");
      return;
    }
    try {
      await quickLaunchClient.launch(app.path);
      await invoke("close_tool_menu_surface");
    } catch (error) {
      setMessage(error instanceof Error && error.message ? error.message : "无法启动该程序。");
    }
  }, [quickLaunch.visiblePinnedApps]);
  const items = toToolMenuPreviewItems(quickLaunch.visiblePinnedApps);
  return (
    <main className={styles.windowRoot} aria-label="快速启动工具盘">
      {quickLaunch.loading && items.length === 0 ? (
        <p className={styles.status}>正在加载固定程序…</p>
      ) : items.length === 0 ? (
        <p className={styles.status}>没有可显示的固定程序</p>
      ) : (
        <ToolMenuPreview variant={nativeLayout} size="settings" items={items} onItemClick={launch} />
      )}
      {message && <p className={styles.error} role="alert">{message}</p>}
    </main>
  );
}
