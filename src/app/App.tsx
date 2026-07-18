import { useEffect, useState } from "react";
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
  createOverviewViewModel,
  EMPTY_OVERVIEW_VIEW_MODEL,
  type OverviewViewModel
} from "./overviewModel";
import { CLIPBOARD_PREVIEW_DATA, OVERVIEW_PREVIEW_DATA } from "./previewData";
import { useDesktopWebViewGuards } from "./useDesktopWebViewGuards";

const routeIds: AppRoute[] = ["overview", "hotkeys", "quickLaunch", "clipboard", "captureQr", "floatingTheme", "general"];

function readInitialRoute(): AppRoute {
  const route = window.location.hash.replace(/^#/, "");
  return routeIds.includes(route as AppRoute) ? (route as AppRoute) : "overview";
}

function App() {
  const [overview, setOverview] = useState<OverviewViewModel>(EMPTY_OVERVIEW_VIEW_MODEL);
  const [route, setRoute] = useState<AppRoute>(readInitialRoute);
  useDesktopWebViewGuards();

  useEffect(() => {
    let active = true;

    void overviewClient
      .load()
      .then((viewModel) => {
        if (active) {
          setOverview(viewModel);
        }
      })
      .catch((error: unknown) => {
        console.error("Unable to load the overview view model", error);
        if (active && import.meta.env.DEV) {
          setOverview(
            createOverviewViewModel({
              ...OVERVIEW_PREVIEW_DATA,
              serviceState: "running"
            })
          );
        }
      });

    return () => {
      active = false;
    };
  }, []);

  function navigate(routeId: AppRoute) {
    setRoute(routeId);
    window.history.replaceState(null, "", `#${routeId}`);
  }

  const page = route;

  const pageContent = (() => {
    switch (page) {
      case "clipboard":
        return <ClipboardPage viewModel={CLIPBOARD_PREVIEW_DATA} />;
      case "hotkeys":
        return <HotkeysPage />;
      case "quickLaunch":
        return <QuickLaunchPage />;
      case "captureQr":
        return <CaptureQrPage />;
      case "floatingTheme":
        return <ThemePage />;
      case "general":
        return <GeneralPage />;
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
      footerVariant={page === "clipboard" ? "clipboard" : "overview"}
    >
      {pageContent}
    </AppShell>
  );
}

export default App;
