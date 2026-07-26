import { useMemo } from "react";
import { themeClient } from "./themeClient";
import { ThemeController } from "./themeController";
import { useControllerStore } from "./useControllerStore";

export function useThemeController() {
  const controller = useMemo(() => new ThemeController(themeClient), []);
  const state = useControllerStore(controller);
  const actions = useMemo(() => ({
    update: controller.update.bind(controller),
    selectBackground: controller.selectBackground.bind(controller),
    removeBackground: controller.removeBackground.bind(controller)
  }), [controller]);

  return { state, ...actions };
}
