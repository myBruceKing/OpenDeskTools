import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  createOverviewViewModel,
  type OverviewBackendSnapshot,
  type OverviewViewModel
} from "./overviewModel";

const OVERVIEW_COMMAND = "get_overview_view_model";
const USAGE_STATISTICS_CHANGED_EVENT = "usage://statistics-changed";

export const overviewClient = {
  async load(): Promise<OverviewViewModel> {
    const snapshot = await invoke<OverviewBackendSnapshot>(OVERVIEW_COMMAND);
    return createOverviewViewModel(snapshot);
  },
  subscribeToUsageChanges(listener: () => void) {
    return listen<unknown>(USAGE_STATISTICS_CHANGED_EVENT, listener);
  }
};
