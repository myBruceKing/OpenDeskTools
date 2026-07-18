import { invoke } from "@tauri-apps/api/core";
import {
  createOverviewViewModel,
  type OverviewBackendSnapshot,
  type OverviewViewModel
} from "./overviewModel";

const OVERVIEW_COMMAND = "get_overview_view_model";

export const overviewClient = {
  async load(): Promise<OverviewViewModel> {
    const snapshot = await invoke<OverviewBackendSnapshot>(OVERVIEW_COMMAND);
    return createOverviewViewModel(snapshot);
  }
};
