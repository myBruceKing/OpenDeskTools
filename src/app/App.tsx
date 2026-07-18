import { useCallback, useEffect, useRef, useState } from "react";
import { AppShell, type AppRoute } from "../components/shell/AppShell";
import { ClipboardPage } from "../pages/clipboard/ClipboardPage";
import { CaptureQrPage } from "../pages/capture-qr/CaptureQrPage";
import { GeneralPage } from "../pages/general/GeneralPage";
import { HotkeysPage } from "../pages/hotkeys/HotkeysPage";
import { OverviewPage } from "../pages/overview/OverviewPage";
import { QuickLaunchPage } from "../pages/quick-launch/QuickLaunchPage";
import { ThemePage } from "../pages/theme/ThemePage";
import { overviewClient } from "./overviewClient";
import {
  EMPTY_OVERVIEW_VIEW_MODEL,
  type OverviewViewModel
} from "./overviewModel";
import { EMPTY_CLIPBOARD_VIEW_MODEL } from "./clipboardModel";
import { useDesktopWebViewGuards } from "./useDesktopWebViewGuards";
import {
  createThemeRootPresentation,
  useDocumentTheme,
  useSystemThemePreferences
} from "./themeRuntime";
import { useThemeController } from "./useThemeController";

const routeIds: AppRoute[] = ["overview", "hotkeys", "quickLaunch", "clipboard", "captureQr", "floatingTheme", "general"];

function readInitialRoute(): AppRoute {
  const route = window.location.hash.replace(/^#/, "");
  return routeIds.includes(route as AppRoute) ? (route as AppRoute) : "overview";
}

function App() {
  const [overview, setOverview] = useState<OverviewViewModel>(EMPTY_OVERVIEW_VIEW_MODEL);
  const [route, setRoute] = useState<AppRoute>(readInitialRoute);
  const overviewRequest = useRef(0);
  const themeController = useThemeController();
  const { systemDark, systemReducedMotion } = useSystemThemePreferences();
  const themePresentation = createThemeRootPresentation(
    themeController.state.current,
    systemDark,
    systemReducedMotion
  );
  useDocumentTheme(themePresentation);
  useDesktopWebViewGuards();

  const refreshOverview = useCallback(async () => {
    const request = ++overviewRequest.current;
    try {
      const viewModel = await overviewClient.load();
      if (request === overviewRequest.current) {
        setOverview(viewModel);
      }
    } catch (error: unknown) {
      console.error("Unable to load the overview view model", error);
      if (request === overviewRequest.current) {
        setOverview(EMPTY_OVERVIEW_VIEW_MODEL);
      }
    }
  }, []);

  useEffect(() => {
    void refreshOverview();

    return () => {
      overviewRequest.current += 1;
    };
  }, [refreshOverview]);

  function navigate(routeId: AppRoute) {
    setRoute(routeId);
    window.history.replaceState(null, "", `#${routeId}`);
  }

  const page = route;

  const pageContent = (() => {
    switch (page) {
      case "clipboard":
        return <ClipboardPage viewModel={EMPTY_CLIPBOARD_VIEW_MODEL} />;
      case "hotkeys":
        return <HotkeysPage onSnapshotChanged={refreshOverview} />;
      case "quickLaunch":
        return <QuickLaunchPage />;
      case "captureQr":
        return <CaptureQrPage />;
      case "floatingTheme":
        return <ThemePage state={themeController.state} onUpdate={themeController.update} />;
      case "general":
        return <GeneralPage version={overview.version} startupEnabled={overview.startupEnabled} />;
      case "overview":
      default:
        return <OverviewPage viewModel={overview} />;
    }
  })();

  return (
    <AppShell
      serviceState={overview.serviceState}
      activeRoute={route}
      onNavigate={navigate}
      theme={themePresentation}
      version={overview.version}
      footerVariant={page === "clipboard" ? "clipboard" : "overview"}
    >
      {pageContent}
    </AppShell>
  );
}

export default App;
