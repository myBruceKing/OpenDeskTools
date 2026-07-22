import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ToolMenuPreview, type ToolMenuPreviewItem } from "../components/patterns/ToolMenuPreview";
import { quickLaunchClient, type QuickLaunchSnapshotPayload, type ToolMenuLayout } from "./quickLaunchClient";
import { toToolMenuPreviewItems, useQuickLaunchViewModel } from "./quickLaunchModel";
import {
  createThemeRootPresentation,
  useDocumentTheme,
  useSystemThemePreferences
} from "./themeRuntime";
import { useDesktopWebViewGuards } from "./useDesktopWebViewGuards";
import { useThemeController } from "./useThemeController";
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
  const themeController = useThemeController();
  const { systemDark, systemReducedMotion } = useSystemThemePreferences();
  const [message, setMessage] = useState<string | null>(null);
  // The WebView is prepared while hidden. It must remain paint-ready even if
  // a first native event arrives before React finishes subscribing; otherwise
  // F2 can reveal a permanently transparent surface.
  const [presentation, setPresentation] = useState<"entering" | "open" | "closing">("open");
  const [nativeLayout, setNativeLayout] = useState<ToolMenuLayout>(nativeToolMenuLayout);
  const [surfaceSnapshot, setSurfaceSnapshot] = useState<QuickLaunchSnapshotPayload | null>(null);
  const [surfaceIcons, setSurfaceIcons] = useState<Map<string, string>>(() => new Map());
  const openingFrame = useRef<number | null>(null);
  const surfaceIconsRef = useRef(surfaceIcons);
  surfaceIconsRef.current = surfaceIcons;
  const theme = createThemeRootPresentation(themeController.state.current, systemDark, systemReducedMotion);
  useDocumentTheme(theme);
  useDesktopWebViewGuards();

  useEffect(() => {
    let active = true;
    const refresh = () => {
      if (active) void quickLaunch.actions.reload();
    };
    const unsubscribe = Promise.all([
      listen("tool-menu://shown", () => {
        refresh();
        if (openingFrame.current !== null) window.cancelAnimationFrame(openingFrame.current);
        setPresentation("entering");
        openingFrame.current = window.requestAnimationFrame(() => {
          if (active) setPresentation("open");
        });
      }),
      listen("tool-menu://closing", () => {
        if (openingFrame.current !== null) window.cancelAnimationFrame(openingFrame.current);
        setPresentation("closing");
      }),
      listen("quick-launch://changed", refresh),
      listen<QuickLaunchSnapshotPayload>("tool-menu://snapshot", ({ payload }) => {
        if (active) {
          quickLaunch.actions.syncSnapshot(payload);
          setSurfaceSnapshot(payload);
          // A direct snapshot is the last-resort opening handshake. Keep the
          // content visible if the subsequent shown event was missed.
          setPresentation("open");
        }
      })
    ]);
    return () => {
      active = false;
      if (openingFrame.current !== null) window.cancelAnimationFrame(openingFrame.current);
      void unsubscribe.then((disposes) => disposes.forEach((dispose) => dispose()));
    };
  }, [quickLaunch.actions]);

  useEffect(() => () => {
    surfaceIconsRef.current.forEach((url) => URL.revokeObjectURL(url));
  }, []);

  useEffect(() => {
    if (!surfaceSnapshot) return undefined;
    let active = true;
    const missing = surfaceSnapshot.pinnedApps.filter((app) => app.visible && app.iconAvailable && !surfaceIconsRef.current.has(app.path));
    if (!missing.length) return () => { active = false; };
    void Promise.all(missing.map(async (app) => {
      try { return [app.path, URL.createObjectURL(await quickLaunchClient.getIcon(app.path))] as const; } catch { return null; }
    })).then((loaded) => {
      if (!active) return;
      const additions = loaded.filter((entry): entry is readonly [string, string] => entry !== null);
      if (!additions.length) return;
      setSurfaceIcons((current) => {
        const next = new Map(current);
        additions.forEach(([path, url]) => next.set(path, url));
        return next;
      });
    });
    return () => { active = false; };
  }, [surfaceSnapshot]);

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

  const displayedApps = useMemo(() => {
    if (!surfaceSnapshot) return quickLaunch.visiblePinnedApps;
    const iconByPath = new Map(quickLaunch.visiblePinnedApps.map((app) => [app.path, app.iconSrc]));
    return surfaceSnapshot.pinnedApps
      .filter((app) => app.visible)
      .map((app) => ({ ...app, iconSrc: surfaceIcons.get(app.path) ?? iconByPath.get(app.path) ?? null }));
  }, [quickLaunch.visiblePinnedApps, surfaceIcons, surfaceSnapshot]);
  const launch = useCallback(async (item: ToolMenuPreviewItem) => {
    const app = displayedApps.find((candidate) => (candidate.id ?? candidate.path) === item.id);
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
  }, [displayedApps]);
  const items = toToolMenuPreviewItems(displayedApps);
  return (
    <main className={styles.windowRoot} data-presentation={presentation} aria-label="快速启动工具盘">
      {quickLaunch.loading && !surfaceSnapshot ? (
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
