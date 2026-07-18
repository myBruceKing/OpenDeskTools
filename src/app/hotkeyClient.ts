import { invoke } from "@tauri-apps/api/core";
import {
  parseHotkeyClassification,
  parseHotkeySnapshot,
  toStableHotkeyActionId,
  type HotkeyClassification,
  type HotkeySnapshot,
  type HotkeyUpdatePatch
} from "./hotkeyModel";

type InvokeFunction = (command: string, args?: Record<string, unknown>) => Promise<unknown>;

export type HotkeyClient = {
  getSnapshot: () => Promise<HotkeySnapshot>;
  classify: (binding: string) => Promise<HotkeyClassification>;
  update: (patch: HotkeyUpdatePatch) => Promise<HotkeySnapshot>;
};

export function createHotkeyClient({
  invokeFunction = invoke as InvokeFunction
}: {
  invokeFunction?: InvokeFunction;
} = {}): HotkeyClient {
  return {
    async getSnapshot() {
      return parseHotkeySnapshot(await invokeFunction("get_hotkey_snapshot"));
    },

    async classify(binding) {
      return parseHotkeyClassification(
        await invokeFunction("classify_hotkey_binding", { binding })
      );
    },

    async update(patch) {
      return parseHotkeySnapshot(
        await invokeFunction("update_hotkey_binding", {
          patch: { ...patch, actionId: toStableHotkeyActionId(patch.actionId) }
        })
      );
    }
  };
}

export const hotkeyClient = createHotkeyClient();
