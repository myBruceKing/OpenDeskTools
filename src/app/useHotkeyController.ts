import { useEffect, useMemo, useSyncExternalStore } from "react";
import { hotkeyClient } from "./hotkeyClient";
import { HotkeyController } from "./hotkeyController";

export function useHotkeyController() {
  const controller = useMemo(() => new HotkeyController(hotkeyClient), []);
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
    openEditor: controller.openEditor.bind(controller),
    closeEditor: controller.closeEditor.bind(controller),
    setBinding: controller.setBinding.bind(controller),
    appendBindingToken: controller.appendBindingToken.bind(controller),
    setForceOverrideSystem: controller.setForceOverrideSystem.bind(controller),
    save: controller.save.bind(controller),
    dismissSystemHotkeyNotice: controller.dismissSystemHotkeyNotice.bind(controller)
  };
}
