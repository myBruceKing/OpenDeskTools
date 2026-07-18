import edgeIcon from "../assets/app-icons/edge.png";
import explorerIcon from "../assets/app-icons/explorer.png";
import powershellIcon from "../assets/app-icons/powershell.png";
import terminalIcon from "../assets/app-icons/terminal.png";
import { useMemo, useSyncExternalStore } from "react";

export type QuickLaunchApp = {
  name: string;
  path: string;
  source?: string;
  iconSrc?: string | null;
};

export const PINNED_QUICK_LAUNCH_APPS: QuickLaunchApp[] = [
  { name: "Cursor", path: "C:\\Users\\Public\\Desktop\\Cursor.lnk" },
  { name: "Visual Studio Code", path: "C:\\Users\\Public\\Desktop\\Visual Studio Code.lnk" },
  {
    name: "终端",
    path: "C:\\Users\\guo\\AppData\\Roaming\\Microsoft\\Windows\\Start Menu\\Programs\\Terminal.lnk",
    iconSrc: terminalIcon
  },
  { name: "文件资源管理器", path: "C:\\Windows\\explorer.exe", iconSrc: explorerIcon },
  {
    name: "Microsoft Edge",
    path: "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
    iconSrc: edgeIcon
  },
  { name: "微信", path: "C:\\Users\\guo\\Desktop\\微信.lnk" }
];

export const DISCOVERED_QUICK_LAUNCH_APPS: QuickLaunchApp[] = [
  { name: "Notepad++", path: "C:\\Program Files\\Notepad++\\notepad++.exe", source: "桌面" },
  {
    name: "PowerShell",
    path: "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe",
    source: "桌面",
    iconSrc: powershellIcon
  },
  {
    name: "GitHub Desktop",
    path: "C:\\Users\\guo\\AppData\\Local\\GitHubDesktop\\GitHubDesktop.exe",
    source: "开始菜单"
  },
  { name: "Snipaste", path: "C:\\Program Files\\Snipaste\\Snipaste.exe", source: "桌面" },
  {
    name: "Obsidian",
    path: "C:\\Users\\guo\\AppData\\Local\\Obsidian\\Obsidian.exe",
    source: "开始菜单"
  }
];

export function toToolMenuPreviewItems(apps: QuickLaunchApp[]) {
  return apps.slice(0, 6).map((app) => ({
    id: app.name,
    label: app.name,
    iconSrc: app.iconSrc
  }));
}

type QuickLaunchState = {
  pinnedApps: QuickLaunchApp[];
  discoveredApps: QuickLaunchApp[];
  visibleAppNames: Set<string>;
};

type QuickLaunchActions = {
  addPinnedApp: (app: QuickLaunchApp) => void;
  reorderPinnedApp: (activeName: string, overName: string) => void;
  setAppVisible: (name: string, visible: boolean) => void;
};

export type QuickLaunchViewModel = QuickLaunchState & {
  visiblePinnedApps: QuickLaunchApp[];
  previewItems: ReturnType<typeof toToolMenuPreviewItems>;
  actions: QuickLaunchActions;
};

let quickLaunchState: QuickLaunchState = {
  pinnedApps: PINNED_QUICK_LAUNCH_APPS,
  discoveredApps: DISCOVERED_QUICK_LAUNCH_APPS,
  visibleAppNames: new Set(PINNED_QUICK_LAUNCH_APPS.map((app) => app.name))
};

const quickLaunchListeners = new Set<() => void>();

function getQuickLaunchState() {
  return quickLaunchState;
}

function subscribeQuickLaunch(listener: () => void) {
  quickLaunchListeners.add(listener);

  return () => {
    quickLaunchListeners.delete(listener);
  };
}

function updateQuickLaunchState(updater: (current: QuickLaunchState) => QuickLaunchState) {
  quickLaunchState = updater(quickLaunchState);
  quickLaunchListeners.forEach((listener) => listener());
}

function reorderApps(apps: QuickLaunchApp[], activeName: string, overName: string) {
  const activeIndex = apps.findIndex((app) => app.name === activeName);
  const overIndex = apps.findIndex((app) => app.name === overName);

  if (activeIndex < 0 || overIndex < 0 || activeIndex === overIndex) {
    return apps;
  }

  const next = [...apps];
  const [active] = next.splice(activeIndex, 1);
  next.splice(overIndex, 0, active);
  return next;
}

const quickLaunchActions: QuickLaunchActions = {
  addPinnedApp(app) {
    const nextApp = {
      ...app,
      name: app.name.trim(),
      path: app.path.trim()
    };

    if (!nextApp.name || !nextApp.path) {
      return;
    }

    updateQuickLaunchState((current) => {
      if (current.pinnedApps.some((pinnedApp) => pinnedApp.name === nextApp.name)) {
        return current;
      }

      const visibleAppNames = new Set(current.visibleAppNames);
      visibleAppNames.add(nextApp.name);

      return {
        ...current,
        pinnedApps: [...current.pinnedApps, nextApp],
        visibleAppNames
      };
    });
  },
  reorderPinnedApp(activeName, overName) {
    updateQuickLaunchState((current) => ({
      ...current,
      pinnedApps: reorderApps(current.pinnedApps, activeName, overName)
    }));
  },
  setAppVisible(name, visible) {
    updateQuickLaunchState((current) => {
      const visibleAppNames = new Set(current.visibleAppNames);

      if (visible) {
        visibleAppNames.add(name);
      } else {
        visibleAppNames.delete(name);
      }

      return {
        ...current,
        visibleAppNames
      };
    });
  }
};

export function useQuickLaunchViewModel(): QuickLaunchViewModel {
  const state = useSyncExternalStore(subscribeQuickLaunch, getQuickLaunchState, getQuickLaunchState);

  return useMemo(() => {
    const visiblePinnedApps = state.pinnedApps.filter((app) => state.visibleAppNames.has(app.name));

    return {
      ...state,
      visiblePinnedApps,
      previewItems: toToolMenuPreviewItems(visiblePinnedApps),
      actions: quickLaunchActions
    };
  }, [state]);
}
