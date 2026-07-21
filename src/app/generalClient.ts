import { invoke } from "@tauri-apps/api/core";
import {
  createGeneralViewModel,
  type GeneralBackendSnapshot,
  type GeneralToggleKind,
  type GeneralViewModel
} from "./generalModel";

const GET_GENERAL_SETTINGS_COMMAND = "get_general_settings";
const OPEN_DATA_DIRECTORY_COMMAND = "open_data_directory";

const TOGGLE_COMMANDS: Record<GeneralToggleKind, string> = {
  autostart: "set_autostart_enabled",
  startMinimized: "set_start_minimized",
  closeToTray: "set_close_to_tray"
};

export const generalClient = {
  async load(): Promise<GeneralViewModel> {
    const snapshot = await invoke<GeneralBackendSnapshot>(GET_GENERAL_SETTINGS_COMMAND);
    return createGeneralViewModel(snapshot);
  },
  async setToggle(kind: GeneralToggleKind, enabled: boolean): Promise<GeneralViewModel> {
    const snapshot = await invoke<GeneralBackendSnapshot>(TOGGLE_COMMANDS[kind], { enabled });
    return createGeneralViewModel(snapshot);
  },
  async openDataDirectory(): Promise<void> {
    await invoke(OPEN_DATA_DIRECTORY_COMMAND);
  }
};
