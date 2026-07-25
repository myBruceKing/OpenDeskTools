import { invoke } from "@tauri-apps/api/core";
import {
  createGeneralViewModel,
  type GeneralBackendSnapshot,
  type GeneralToggleKind,
  type GeneralViewModel
} from "./generalModel";

const GET_GENERAL_SETTINGS_COMMAND = "get_general_settings";
const SELECT_AND_MIGRATE_DATA_DIRECTORY_COMMAND = "select_and_migrate_data_directory";
const RESTART_AS_ADMINISTRATOR_COMMAND = "restart_as_administrator";

const TOGGLE_COMMANDS: Record<GeneralToggleKind, string> = {
  autostart: "set_autostart_enabled",
  startMinimized: "set_start_minimized",
  closeToTray: "set_close_to_tray",
  trayIconVisible: "set_tray_icon_visible",
  crashDiagnostics: "set_crash_diagnostics_enabled"
};

export type DataDirectoryMigrationResult = {
  dataDirectory: string;
  restartRequired: boolean;
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
  async selectAndMigrateDataDirectory(): Promise<DataDirectoryMigrationResult | null> {
    return invoke<DataDirectoryMigrationResult | null>(SELECT_AND_MIGRATE_DATA_DIRECTORY_COMMAND);
  },
  async restartAsAdministrator(): Promise<void> {
    return invoke<void>(RESTART_AS_ADMINISTRATOR_COMMAND);
  }
};
