import { useEffect, useMemo, useRef, useState } from "react";
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
  const [icons, setIcons] = useState<Map<string, string>>(() => new Map());
  const iconsRef = useRef(icons);
  iconsRef.current = icons;

  const apply = async (next: Promise<QuickLaunchSnapshotPayload>) => {
    setLoading(true);
    try {
      const result = await next;
      setSnapshot(withIcons(result, icons));
      setError(null);
      // A discovered list can contain hundreds of entries. Loading all Shell
      // icons at once blocks interaction and was the cause of the apparent
      // page freeze; fixed apps are few and visible immediately.
      const apps = result.pinnedApps.filter((app) => app.iconAvailable && !icons.has(app.path));
      if (apps.length) {
        void Promise.all(apps.map(async (app) => {
          try { return [app.path, URL.createObjectURL(await quickLaunchClient.getIcon(app.path))] as const; } catch { return null; }
        })).then((loaded) => {
          const additions = loaded.filter((entry): entry is readonly [string, string] => entry !== null);
          if (!additions.length) return;
          setIcons((current) => {
            const merged = new Map(current);
            additions.forEach(([path, url]) => merged.set(path, url));
            setSnapshot((currentSnapshot) => ({
              pinnedApps: currentSnapshot.pinnedApps.map((app) => ({ ...app, iconSrc: merged.get(app.path) ?? null })),
              discoveredApps: currentSnapshot.discoveredApps.map((app) => ({ ...app, iconSrc: merged.get(app.path) ?? null })),
              toolMenu: currentSnapshot.toolMenu
            }));
            return merged;
          });
        });
      }
    } catch (cause) {
      setError(messageFor(cause));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => { void apply(quickLaunchClient.getSnapshot()); }, []); // initial native snapshot
  useEffect(() => () => { iconsRef.current.forEach((url) => URL.revokeObjectURL(url)); }, []);

  return useMemo(() => {
    const visiblePinnedApps = snapshot.pinnedApps.filter((app) => app.visible);
    const run = (operation: Promise<QuickLaunchSnapshotPayload>) => apply(operation);
    return {
      sourceAvailable: true,
      loading, error, pinnedApps: snapshot.pinnedApps, discoveredApps: snapshot.discoveredApps, visiblePinnedApps,
      previewItems: toToolMenuPreviewItems(visiblePinnedApps), toolMenu: snapshot.toolMenu,
      actions: {
        syncSnapshot: (next) => { void apply(Promise.resolve(next)); },
        reload: () => run(quickLaunchClient.getSnapshot()),
        refresh: () => run(quickLaunchClient.rescan()),
        addPinnedApp: (app) => run(quickLaunchClient.pin(app.path, app.source)),
        addManually: async () => { const path = await quickLaunchClient.selectFile(); if (path) await run(quickLaunchClient.pin(path, "手动添加")); },
        removePinnedApp: (path) => run(quickLaunchClient.unpin(path)),
        reorderPinnedApp: (activePath, overPath) => run(quickLaunchClient.reorder(activePath, overPath)),
        setAppVisible: (path, visible) => run(quickLaunchClient.setVisible(path, visible)),
        updateToolMenu: (preferences) => run(quickLaunchClient.updateToolMenu(preferences)),
        launchApp: async (path) => { try { await quickLaunchClient.launch(path); setError(null); } catch (cause) { setError(messageFor(cause)); } }
      }
    };
  }, [error, icons, loading, snapshot]);
}
