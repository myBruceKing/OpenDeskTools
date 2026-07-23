import { useEffect, useState } from "react";
import { QrConversionToast } from "../components/patterns/QrConversionToast";
import {
  parseQrConversionFeedback,
  type QrConversionFeedback
} from "./qrClient";
import {
  createThemeRootPresentation,
  useDocumentTheme,
  useSystemThemePreferences
} from "./themeRuntime";
import { useDesktopWebViewGuards } from "./useDesktopWebViewGuards";
import { useThemeController } from "./useThemeController";

declare global {
  interface Window {
    __OPENDESK_QR_FEEDBACK?: unknown;
  }
}

export function QrToastSurfaceRoot() {
  const themeController = useThemeController();
  const { systemDark, systemReducedMotion } = useSystemThemePreferences();
  const theme = createThemeRootPresentation(
    themeController.state.current,
    systemDark,
    systemReducedMotion
  );
  const [feedback, setFeedback] = useState<QrConversionFeedback | null>(() =>
    parseQrConversionFeedback(window.__OPENDESK_QR_FEEDBACK)
  );
  useDocumentTheme(theme);
  useDesktopWebViewGuards();

  useEffect(() => {
    const sync = () => {
      const next = parseQrConversionFeedback(window.__OPENDESK_QR_FEEDBACK);
      if (next) setFeedback(next);
    };
    window.addEventListener("opendesk-qr-feedback", sync);
    sync();
    return () => window.removeEventListener("opendesk-qr-feedback", sync);
  }, []);

  return <QrConversionToast feedback={feedback} />;
}
