import { useEffect, useMemo, useSyncExternalStore } from "react";
import { clipboardClient } from "./clipboardClient";
import { ClipboardController } from "./clipboardController";

export function useClipboardController(surfaceActiveHint = false) {
  const controller = useMemo(
    () => new ClipboardController(clipboardClient, surfaceActiveHint),
    []
  );
  const state = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot,
    controller.getSnapshot
  );

  useEffect(() => {
    controller.start();
    return () => controller.stop();
  }, [controller]);

  useEffect(() => {
    controller.setSurfaceActiveHint(surfaceActiveHint);
  }, [controller, surfaceActiveHint]);

  const actions = useMemo(() => ({
    loadImage: clipboardClient.getImage,
    loadSourceIcon: clipboardClient.getSourceIcon,
    copyItem: controller.copyItem.bind(controller),
    inputItem: controller.inputItem.bind(controller),
    setMonitoring: controller.setMonitoring.bind(controller),
    updateSettings: controller.updateSettings.bind(controller),
    closeSurface: () => controller.closeSurface(),
    setFavorite: controller.setFavorite.bind(controller),
    updateText: controller.updateText.bind(controller),
    deleteItem: controller.deleteItem.bind(controller),
    clearUnfavoriteHistory: controller.clearUnfavoriteHistory.bind(controller)
  }), [controller]);

  return { state, ...actions };
}
