import { invoke } from "@tauri-apps/api/core";
import {
  createOverviewViewModel,
  type OverviewBackendSnapshot,
  type OverviewViewModel
} from "./overviewModel";
import { OVERVIEW_PREVIEW_DATA } from "./previewData";

const OVERVIEW_COMMAND = "get_overview_view_model";

export const overviewClient = {
  async load(): Promise<OverviewViewModel> {
    const snapshot = await invoke<OverviewBackendSnapshot>(OVERVIEW_COMMAND);
    const presentationSnapshot = import.meta.env.DEV
      ? {
          ...snapshot,
          version: OVERVIEW_PREVIEW_DATA.version,
          startupEnabled: OVERVIEW_PREVIEW_DATA.startupEnabled,
          hotkeys: snapshot.hotkeys ?? OVERVIEW_PREVIEW_DATA.hotkeys,
          statistics: snapshot.statistics ?? OVERVIEW_PREVIEW_DATA.statistics
        }
      : snapshot;

    return createOverviewViewModel(presentationSnapshot);
  }
};
