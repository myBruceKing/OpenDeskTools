import { useMemo } from "react";
import { hotkeyClient } from "./hotkeyClient";
import { HotkeyController } from "./hotkeyController";
import { useControllerStore } from "./useControllerStore";

export function useHotkeyController() {
  const controller = useMemo(() => new HotkeyController(hotkeyClient), []);
  const state = useControllerStore(controller);
  const actions = useMemo(() => ({
    openEditor: controller.openEditor.bind(controller),
    closeEditor: controller.closeEditor.bind(controller),
    setBinding: controller.setBinding.bind(controller),
    appendBindingToken: controller.appendBindingToken.bind(controller),
    setForceOverrideSystem: controller.setForceOverrideSystem.bind(controller),
    setEnabled: controller.setEnabled.bind(controller),
    save: controller.save.bind(controller),
    dismissSystemHotkeyNotice: controller.dismissSystemHotkeyNotice.bind(controller)
  }), [controller]);

  return { state, ...actions };
}
