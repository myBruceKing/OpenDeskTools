import { useEffect, useMemo, useSyncExternalStore } from "react";
import { clipboardClient } from "./clipboardClient";
import { ClipboardController } from "./clipboardController";

export function useClipboardController() {
  const controller = useMemo(() => new ClipboardController(clipboardClient), []);
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
    setFavorite: controller.setFavorite.bind(controller),
    deleteItem: controller.deleteItem.bind(controller),
    clearUnfavoriteHistory: controller.clearUnfavoriteHistory.bind(controller)
  };
}
