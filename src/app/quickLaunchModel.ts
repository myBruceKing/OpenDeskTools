import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { quickLaunchClient, type QuickLaunchAppPayload, type QuickLaunchSnapshotPayload, type ToolMenuPreferences } from "./quickLaunchClient";

export type QuickLaunchApp = {
  name: string;
  path: string;
  arguments?: string;
  workingDirectory?: string | null;
  iconPath?: string;
  iconIndex?: number;
  source?: string;
  id?: string;
  visible?: boolean;
  available?: boolean;
  iconAvailable?: boolean;
  iconSrc?: string | null;
};

export const PINNED_QUICK_LAUNCH_APPS: QuickLaunchApp[] = [];
export const DISCOVERED_QUICK_LAUNCH_APPS: QuickLaunchApp[] = [];

export function toToolMenuPreviewItems(apps: QuickLaunchApp[]) {
  return apps.map((app) => ({ id: app.id ?? app.path, label: app.name, iconSrc: app.iconSrc }));
}

type QuickLaunchActions = {
  syncSnapshot: (snapshot: QuickLaunchSnapshotPayload) => void;
  reload: () => Promise<void>;
  refresh: () => Promise<void>;
  addPinnedApp: (app: Pick<QuickLaunchApp, "path" | "source">) => Promise<void>;
  addManually: () => Promise<void>;
  removePinnedApp: (path: string) => Promise<void>;
  reorderPinnedApp: (activePath: string, overPath: string) => Promise<void>;
  swapPinnedApps: (activePath: string, overPath: string) => Promise<void>;
  setAppVisible: (path: string, visible: boolean) => Promise<void>;
  launchApp: (path: string) => Promise<void>;
  updateToolMenu: (preferences: ToolMenuPreferences) => Promise<void>;
};

export type QuickLaunchViewModel = {
  sourceAvailable: boolean;
  loading: boolean;
  error: string | null;
  pinnedApps: QuickLaunchApp[];
  discoveredApps: QuickLaunchApp[];
  visiblePinnedApps: QuickLaunchApp[];
  previewItems: ReturnType<typeof toToolMenuPreviewItems>;
  toolMenu: ToolMenuPreferences;
  actions: QuickLaunchActions;
};

function messageFor(error: unknown) {
  return error instanceof Error && error.message ? error.message : "快速启动操作未完成，请重试。";
}

function withIcons(snapshot: QuickLaunchSnapshotPayload, previous: Map<string, string>) {
  const decorate = (app: QuickLaunchAppPayload): QuickLaunchApp => ({ ...app, iconSrc: previous.get(app.path) ?? null });
  return { pinnedApps: snapshot.pinnedApps.map(decorate), discoveredApps: snapshot.discoveredApps.map(decorate), toolMenu: snapshot.toolMenu };
}

export function useQuickLaunchViewModel(): QuickLaunchViewModel {
  const [snapshot, setSnapshot] = useState<{ pinnedApps: QuickLaunchApp[]; discoveredApps: QuickLaunchApp[]; toolMenu: ToolMenuPreferences }>({ pinnedApps: [], discoveredApps: [], toolMenu: { layout: "wheel", keepOpenOnKeyRelease: false } });
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const iconsRef = useRef(new Map<string, string>());
  const loadingIconsRef = useRef(new Set<string>());
  const mountedRef = useRef(true);

  const loadPinnedIcons = useCallback((next: QuickLaunchSnapshotPayload) => {
    // A discovered list can contain hundreds of entries. The shared model
    // eagerly loads only the few pinned icons; discovered rows lazy-load their
    // own icon when they enter the viewport.
    const apps = next.pinnedApps.filter((app) =>
      app.iconAvailable
      && !iconsRef.current.has(app.path)
      && !loadingIconsRef.current.has(app.path)
    );
    apps.forEach((app) => loadingIconsRef.current.add(app.path));
    if (!apps.length) return;
    void Promise.all(apps.map(async (app) => {
      try {
        return [app.path, URL.createObjectURL(await quickLaunchClient.getIcon(app.path))] as const;
      } catch {
        return null;
      }
    })).then((loaded) => {
      apps.forEach((app) => loadingIconsRef.current.delete(app.path));
      const additions = loaded.filter((entry): entry is readonly [string, string] => entry !== null);
      if (!mountedRef.current) {
        additions.forEach(([, url]) => URL.revokeObjectURL(url));
        return;
      }
      additions.forEach(([path, url]) => iconsRef.current.set(path, url));
      if (additions.length) {
        setSnapshot((current) => ({
          ...current,
          pinnedApps: current.pinnedApps.map((app) => ({ ...app, iconSrc: iconsRef.current.get(app.path) ?? null })),
          discoveredApps: current.discoveredApps.map((app) => ({ ...app, iconSrc: iconsRef.current.get(app.path) ?? null }))
        }));
      }
    });
  }, []);

  const commitSnapshot = useCallback((next: QuickLaunchSnapshotPayload) => {
    setSnapshot(withIcons(next, iconsRef.current));
    setError(null);
    loadPinnedIcons(next);
  }, [loadPinnedIcons]);

  const run = useCallback(async (operation: () => Promise<QuickLaunchSnapshotPayload>) => {
    setLoading(true);
    try {
      commitSnapshot(await operation());
    } catch (cause) {
      setError(messageFor(cause));
    } finally {
      setLoading(false);
    }
  }, [commitSnapshot]);

  const reload = useCallback(() => run(() => quickLaunchClient.getSnapshot()), [run]);
  const refresh = useCallback(() => run(() => quickLaunchClient.rescan()), [run]);
  const addPinnedApp = useCallback((app: Pick<QuickLaunchApp, "path" | "source">) =>
    run(() => quickLaunchClient.pin(app.path, app.source)), [run]);
  const addManually = useCallback(async () => {
    const path = await quickLaunchClient.selectFile();
    if (path) await run(() => quickLaunchClient.pin(path, "手动添加"));
  }, [run]);
  const removePinnedApp = useCallback((path: string) => run(() => quickLaunchClient.unpin(path)), [run]);
  const reorderPinnedApp = useCallback((activePath: string, overPath: string) =>
    run(() => quickLaunchClient.reorder(activePath, overPath)), [run]);
  const swapPinnedApps = useCallback((activePath: string, overPath: string) =>
    run(() => quickLaunchClient.swap(activePath, overPath)), [run]);
  const setAppVisible = useCallback((path: string, visible: boolean) =>
    run(() => quickLaunchClient.setVisible(path, visible)), [run]);
  const updateToolMenu = useCallback((preferences: ToolMenuPreferences) =>
    run(() => quickLaunchClient.updateToolMenu(preferences)), [run]);
  const syncSnapshot = useCallback((next: QuickLaunchSnapshotPayload) => commitSnapshot(next), [commitSnapshot]);
  const launchApp = useCallback(async (path: string) => {
    try {
      await quickLaunchClient.launch(path);
      setError(null);
    } catch (cause) {
      setError(messageFor(cause));
    }
  }, []);

  const actions = useMemo<QuickLaunchActions>(() => ({
    syncSnapshot,
    reload,
    refresh,
    addPinnedApp,
    addManually,
    removePinnedApp,
    reorderPinnedApp,
    swapPinnedApps,
    setAppVisible,
    launchApp,
    updateToolMenu
  }), [
    addManually,
    addPinnedApp,
    launchApp,
    refresh,
    reload,
    removePinnedApp,
    reorderPinnedApp,
    setAppVisible,
    swapPinnedApps,
    syncSnapshot,
    updateToolMenu
  ]);

  useEffect(() => { void reload(); }, [reload]);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      iconsRef.current.forEach((url) => URL.revokeObjectURL(url));
      iconsRef.current.clear();
    };
  }, []);

  return useMemo(() => {
    const visiblePinnedApps = snapshot.pinnedApps.filter((app) => app.visible);
    return {
      sourceAvailable: true,
      loading, error, pinnedApps: snapshot.pinnedApps, discoveredApps: snapshot.discoveredApps, visiblePinnedApps,
      previewItems: toToolMenuPreviewItems(visiblePinnedApps), toolMenu: snapshot.toolMenu,
      actions
    };
  }, [actions, error, loading, snapshot]);
}
