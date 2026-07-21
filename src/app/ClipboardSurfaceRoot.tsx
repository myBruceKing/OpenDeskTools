import { useRef } from "react";
import { ClipboardSurface } from "../surfaces/clipboard/ClipboardSurface";
import { clipboardClient } from "./clipboardClient";
import { useClipboardSurfaceUnderlayColor } from "./clipboardSurfaceUnderlay";
import {
  createThemeRootPresentation,
  useDocumentTheme,
  useSystemThemePreferences
} from "./themeRuntime";
import { useClipboardController } from "./useClipboardController";
import { useDesktopWebViewGuards } from "./useDesktopWebViewGuards";
import { useThemeController } from "./useThemeController";
import { useWindowSurfaceMetricsTrace } from "./windowSurfaceMetrics";
import styles from "./ClipboardSurfaceRoot.module.css";

export function ClipboardSurfaceRoot() {
  const windowRootRef = useRef<HTMLDivElement>(null);
  const clipboard = useClipboardController(true);
  const themeController = useThemeController();
  const { systemDark, systemReducedMotion } = useSystemThemePreferences();
  const theme = createThemeRootPresentation(
    themeController.state.current,
    systemDark,
    systemReducedMotion
  );
  useDocumentTheme(theme);
  useDesktopWebViewGuards();
  useClipboardSurfaceUnderlayColor(theme.resolvedTheme, clipboardClient.setSurfaceUnderlayColor);
  useWindowSurfaceMetricsTrace("clipboard", windowRootRef, clipboardClient.tracePreviewDebug);

  return (
    <div ref={windowRootRef} className={styles.windowRoot}>
      <ClipboardSurface
        state={clipboard.state}
        loadSourceIcon={clipboard.loadSourceIcon}
        onCopy={clipboard.copyItem}
        onInput={clipboard.inputItem}
        onClose={clipboard.closeSurface}
        onSetFavorite={clipboard.setFavorite}
        onDelete={clipboard.deleteItem}
        onOpenPreview={clipboardClient.openPreviewSurface}
        onClosePreview={clipboardClient.closePreviewSurface}
        onSubscribePreviewHover={clipboardClient.subscribePreviewHover}
        onTracePreviewDebug={clipboardClient.tracePreviewDebug}
      />
    </div>
  );
}
