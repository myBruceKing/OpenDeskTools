import { useCallback, useEffect, useRef, useState } from "react";
import { generalClient } from "./generalClient";
import {
  EMPTY_GENERAL_VIEW_MODEL,
  parseGeneralCommandError,
  type GeneralToggleKind,
  type GeneralViewModel
} from "./generalModel";

export type GeneralSettingsState = {
  viewModel: GeneralViewModel;
  loaded: boolean;
  /** The toggle currently being persisted, or `null` when idle. */
  pending: GeneralToggleKind | "dataDirectory" | null;
  error: string | null;
  dataDirectoryMigration: { dataDirectory: string; restartRequired: boolean } | null;
};

const INITIAL_STATE: GeneralSettingsState = {
  viewModel: EMPTY_GENERAL_VIEW_MODEL,
  loaded: false,
  pending: null,
  error: null,
  dataDirectoryMigration: null
};

export function useGeneralSettings() {
  const [state, setState] = useState<GeneralSettingsState>(INITIAL_STATE);
  const loadRequest = useRef(0);

  const refresh = useCallback(async () => {
    const request = ++loadRequest.current;
    try {
      const viewModel = await generalClient.load();
      if (request === loadRequest.current) {
        setState((previous) => ({
          ...previous,
          viewModel,
          loaded: true,
          error: null
        }));
      }
    } catch (error: unknown) {
      console.error("Unable to load the general settings view model", error);
      if (request === loadRequest.current) {
        setState((previous) => ({
          ...previous,
          viewModel: EMPTY_GENERAL_VIEW_MODEL,
          loaded: true
        }));
      }
    }
  }, []);

  useEffect(() => {
    void refresh();
    return () => {
      loadRequest.current += 1;
    };
  }, [refresh]);

  const setToggle = useCallback(async (kind: GeneralToggleKind, enabled: boolean) => {
    setState((previous) => ({ ...previous, pending: kind, error: null }));
    try {
      const viewModel = await generalClient.setToggle(kind, enabled);
      setState((previous) => ({ ...previous, viewModel, pending: null, error: null }));
    } catch (error: unknown) {
      setState((previous) => ({
        ...previous,
        pending: null,
        error: parseGeneralCommandError(error)
      }));
    }
  }, []);

  const selectAndMigrateDataDirectory = useCallback(async () => {
    setState((previous) => ({ ...previous, pending: "dataDirectory", error: null }));
    try {
      const result = await generalClient.selectAndMigrateDataDirectory();
      setState((previous) => ({
        ...previous,
        pending: null,
        error: null,
        dataDirectoryMigration: result
      }));
    } catch (error: unknown) {
      setState((previous) => ({
        ...previous,
        pending: null,
        error: parseGeneralCommandError(error)
      }));
    }
  }, []);

  return { state, setToggle, selectAndMigrateDataDirectory, refresh };
}
