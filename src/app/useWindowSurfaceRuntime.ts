import {
  createThemeRootPresentation,
  useDocumentTheme,
  useSystemThemePreferences
} from "./themeRuntime";
import { useDesktopWebViewGuards } from "./useDesktopWebViewGuards";
import { useThemeController } from "./useThemeController";

export function useWindowSurfaceRuntime() {
  const themeController = useThemeController();
  const { systemDark, systemReducedMotion } = useSystemThemePreferences();
  const theme = createThemeRootPresentation(
    themeController.state.current,
    systemDark,
    systemReducedMotion
  );

  useDocumentTheme(theme);
  useDesktopWebViewGuards();

  return theme;
}
