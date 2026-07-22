export type GeneralBackendSnapshot = {
  version: string;
  autostartEnabled: boolean;
  startMinimized: boolean;
  closeToTray: boolean;
  crashDiagnosticsEnabled: boolean;
  dataDirectory: string;
};

export type GeneralViewModel = {
  version: string | null;
  autostartEnabled: boolean | null;
  startMinimized: boolean | null;
  closeToTray: boolean | null;
  crashDiagnosticsEnabled: boolean | null;
  dataDirectory: string | null;
};

export function createGeneralViewModel(
  snapshot: GeneralBackendSnapshot | null
): GeneralViewModel {
  return {
    version: snapshot?.version ?? null,
    autostartEnabled: snapshot?.autostartEnabled ?? null,
    startMinimized: snapshot?.startMinimized ?? null,
    closeToTray: snapshot?.closeToTray ?? null,
    crashDiagnosticsEnabled: snapshot?.crashDiagnosticsEnabled ?? null,
    dataDirectory: snapshot?.dataDirectory ?? null
  };
}

/** Behaviour toggles the general page can persist to the backend. */
export type GeneralToggleKind = "autostart" | "startMinimized" | "closeToTray" | "crashDiagnostics";

export const EMPTY_GENERAL_VIEW_MODEL = createGeneralViewModel(null);

/**
 * Extracts a human-readable message from a rejected general command. The
 * backend rejects with `{ code, message }`; anything else falls back to a
 * stable Chinese default so the UI never surfaces `[object Object]`.
 */
export function parseGeneralCommandError(error: unknown): string {
  if (
    error &&
    typeof error === "object" &&
    "message" in error &&
    typeof (error as { message: unknown }).message === "string"
  ) {
    return (error as { message: string }).message;
  }
  if (error instanceof Error && error.message) {
    return error.message;
  }
  return "设置未生效，请重试。";
}
