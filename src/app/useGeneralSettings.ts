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
  loadStatus: "loading" | "ready" | "error";
  loadError: string | null;
  /** The toggle currently being persisted, or `null` when idle. */
  pending: GeneralToggleKind | "dataDirectory" | "administratorRestart" | null;
  error: string | null;
  dataDirectoryMigration: { dataDirectory: string; restartRequired: boolean } | null;
};

const INITIAL_STATE: GeneralSettingsState = {
  viewModel: EMPTY_GENERAL_VIEW_MODEL,
  loadStatus: "loading",
  loadError: null,
  pending: null,
  error: null,
  dataDirectoryMigration: null
};

export function useGeneralSettings() {
  const [state, setState] = useState<GeneralSettingsState>(INITIAL_STATE);
  const loadRequest = useRef(0);
  const ready = useRef(false);
  const mutationPending = useRef(false);

  const refresh = useCallback(async () => {
    if (mutationPending.current) return;
    const request = ++loadRequest.current;
    ready.current = false;
    setState((previous) => ({ ...previous, loadStatus: "loading", error: null }));
    try {
      const viewModel = await generalClient.load();
      if (request === loadRequest.current) {
        ready.current = true;
        setState((previous) => ({
          ...previous,
          viewModel,
          loadStatus: "ready",
          loadError: null,
          error: null
        }));
      }
    } catch (error: unknown) {
      if (request === loadRequest.current) {
        setState((previous) => ({
          ...previous,
          viewModel: EMPTY_GENERAL_VIEW_MODEL,
          loadStatus: "error",
          loadError: typeof error === "object" && error !== null && "message" in error
            ? parseGeneralCommandError(error)
            : "暂时无法读取常规设置，请重试。"
        }));
      }
    }
  }, []);

  useEffect(() => {
    void refresh();
    return () => {
      loadRequest.current += 1;
      ready.current = false;
      mutationPending.current = false;
    };
  }, [refresh]);

  const setToggle = useCallback(async (kind: GeneralToggleKind, enabled: boolean) => {
    if (!ready.current || mutationPending.current) return;
    mutationPending.current = true;
    const request = loadRequest.current;
    setState((previous) => ({ ...previous, pending: kind, error: null }));
    try {
      const viewModel = await generalClient.setToggle(kind, enabled);
      if (request !== loadRequest.current) return;
      setState((previous) => ({ ...previous, viewModel, pending: null, error: null }));
    } catch (error: unknown) {
      if (request !== loadRequest.current) return;
      setState((previous) => ({
        ...previous,
        pending: null,
        error: parseGeneralCommandError(error)
      }));
    } finally {
      if (request === loadRequest.current) mutationPending.current = false;
    }
  }, []);

  const selectAndMigrateDataDirectory = useCallback(async () => {
    if (!ready.current || mutationPending.current) return;
    mutationPending.current = true;
    const request = loadRequest.current;
    setState((previous) => ({ ...previous, pending: "dataDirectory", error: null }));
    try {
      const result = await generalClient.selectAndMigrateDataDirectory();
      if (request !== loadRequest.current) return;
      setState((previous) => ({
        ...previous,
        pending: null,
        error: null,
        dataDirectoryMigration: result
      }));
    } catch (error: unknown) {
      if (request !== loadRequest.current) return;
      setState((previous) => ({
        ...previous,
        pending: null,
        error: parseGeneralCommandError(error)
      }));
    } finally {
      if (request === loadRequest.current) mutationPending.current = false;
    }
  }, []);

  const restartAsAdministrator = useCallback(async () => {
    if (!ready.current || mutationPending.current) return;
    mutationPending.current = true;
    const request = loadRequest.current;
    setState((previous) => ({ ...previous, pending: "administratorRestart", error: null }));
    try {
      await generalClient.restartAsAdministrator();
    } catch (error: unknown) {
      if (request !== loadRequest.current) return;
      mutationPending.current = false;
      setState((previous) => ({
        ...previous,
        pending: null,
        error: parseGeneralCommandError(error)
      }));
    }
  }, []);

  return {
    state,
    setToggle,
    selectAndMigrateDataDirectory,
    restartAsAdministrator,
    refresh
  };
}
