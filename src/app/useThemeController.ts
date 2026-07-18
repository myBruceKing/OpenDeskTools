import { useEffect, useMemo, useSyncExternalStore } from "react";
import { themeClient } from "./themeClient";
import { ThemeController } from "./themeController";

export function useThemeController() {
  const controller = useMemo(() => new ThemeController(themeClient), []);
  const state = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot,
    controller.getSnapshot
  );

  useEffect(() => {
    controller.start();
    return () => controller.stop();
  }, [controller]);

  return {
    state,
    update: controller.update.bind(controller)
  };
}
