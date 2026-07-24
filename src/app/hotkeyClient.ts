import { invoke } from "@tauri-apps/api/core";
import {
  parseHotkeyClassification,
  parseHotkeySnapshot,
  parseHotkeyUpdateResult,
  toStableHotkeyActionId,
  type HotkeyClassification,
  type HotkeyEnabledPatch,
  type HotkeySnapshot,
  type HotkeyUpdatePatch,
  type HotkeyUpdateResult
} from "./hotkeyModel";

type InvokeFunction = (command: string, args?: Record<string, unknown>) => Promise<unknown>;

export type HotkeyClient = {
  getSnapshot: () => Promise<HotkeySnapshot>;
  classify: (binding: string) => Promise<HotkeyClassification>;
  update: (patch: HotkeyUpdatePatch) => Promise<HotkeyUpdateResult>;
  updateEnabled: (patch: HotkeyEnabledPatch) => Promise<HotkeyUpdateResult>;
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
      return parseHotkeyUpdateResult(
        await invokeFunction("update_hotkey_binding", {
          patch: { ...patch, actionId: toStableHotkeyActionId(patch.actionId) }
        })
      );
    },

    async updateEnabled(patch) {
      return parseHotkeyUpdateResult(
        await invokeFunction("update_hotkey_enabled", {
          patch: { ...patch, actionId: toStableHotkeyActionId(patch.actionId) }
        })
      );
    }
  };
}

export const hotkeyClient = createHotkeyClient();
