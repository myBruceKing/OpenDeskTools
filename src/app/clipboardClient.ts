import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import {
  parseClipboardClearResult,
  parseClipboardDeleteResult,
  parseClipboardHistoryItem,
  parseClipboardHistoryResult,
  parseClipboardItemActionResult,
  parseClipboardSurfaceCloseResult,
  type ClipboardHistoryItem,
  type ClipboardHistoryQuery,
  type ClipboardHistoryResult,
  type ClipboardItemActionResult,
  type ClipboardSurfaceCloseResult
} from "./clipboardModel";

type InvokeFunction = (command: string, args?: Record<string, unknown>) => Promise<unknown>;
type ListenFunction = (
  event: string,
  handler: (event: { payload: unknown }) => void
) => Promise<() => void>;
type EmitFunction = (event: string, payload?: unknown) => Promise<void>;

export type ClipboardPreviewSurfaceState = {
  recordId: string | null;
  visible: boolean;
};

export type ClipboardPreviewSurfaceChange = ClipboardPreviewSurfaceState & {
  change: "opened" | "selection_changed" | "closed";
};

export type ClipboardPreviewHoverChange = {
  inside: boolean;
  recordId: string | null;
};

export type ClipboardPreviewDebugEvent =
  | "open_requested"
  | "open_resolved"
  | "open_failed"
  | "close_scheduled"
  | "close_canceled"
  | "close_fired"
  | "close_requested"
  | "close_resolved"
  | "close_failed"
  | "hover_inside"
  | "hover_outside"
  | "window_blur_ignored"
  | "surface_metrics";

function parseClipboardPreviewSurfaceState(value: unknown): ClipboardPreviewSurfaceState {
  if (!value || typeof value !== "object") throw new Error("Invalid clipboard preview surface payload");
  const payload = value as Record<string, unknown>;
  if ((payload.recordId !== null && typeof payload.recordId !== "string") || typeof payload.visible !== "boolean") {
    throw new Error("Invalid clipboard preview surface payload");
  }
  return { recordId: payload.recordId as string | null, visible: payload.visible };
}

function parseClipboardPreviewSurfaceChange(value: unknown): ClipboardPreviewSurfaceChange {
  const state = parseClipboardPreviewSurfaceState(value);
  const change = (value as Record<string, unknown>).change;
  if (change !== "opened" && change !== "selection_changed" && change !== "closed") {
    throw new Error("Invalid clipboard preview change payload");
  }
  return { ...state, change };
}

function parseClipboardPreviewHoverChange(value: unknown): ClipboardPreviewHoverChange {
  if (!value || typeof value !== "object") throw new Error("Invalid clipboard preview hover payload");
  const payload = value as Record<string, unknown>;
  if (typeof payload.inside !== "boolean" || (payload.recordId !== null && typeof payload.recordId !== "string")) {
    throw new Error("Invalid clipboard preview hover payload");
  }
  return { inside: payload.inside, recordId: payload.recordId as string | null };
}

export type ClipboardClient = {
  getHistory: (query: ClipboardHistoryQuery) => Promise<ClipboardHistoryResult>;
  getImage: (id: string) => Promise<Blob>;
  getSourceIcon: (id: string) => Promise<Blob>;
  copyItem: (id: string) => Promise<ClipboardItemActionResult>;
  inputItem: (id: string) => Promise<ClipboardItemActionResult>;
  closeSurface: () => Promise<ClipboardSurfaceCloseResult>;
  openPreviewSurface: (recordId: string) => Promise<void>;
  closePreviewSurface: () => Promise<void>;
  getPreviewSurfaceState: () => Promise<ClipboardPreviewSurfaceState>;
  subscribePreviewSurface: (listener: (change: ClipboardPreviewSurfaceChange) => void) => Promise<() => void>;
  publishPreviewHover: (change: ClipboardPreviewHoverChange) => Promise<void>;
  subscribePreviewHover: (listener: (change: ClipboardPreviewHoverChange) => void) => Promise<() => void>;
  tracePreviewDebug: (
    event: ClipboardPreviewDebugEvent,
    recordId?: string | null,
    detail?: string | null
  ) => Promise<void>;
  setSurfaceUnderlayColor: (color: string) => Promise<void>;
  setFavorite: (id: string, isFavorite: boolean) => Promise<ClipboardHistoryItem>;
  updateText: (id: string, textContent: string, expectedRevision: number) => Promise<ClipboardHistoryItem>;
  deleteItem: (id: string) => Promise<{ deleted: boolean }>;
  clearUnfavoriteHistory: () => Promise<{ deletedCount: number }>;
  subscribe: (listener: () => void) => Promise<() => void>;
};

export function createClipboardClient({
  invokeFunction = invoke as InvokeFunction,
  listenFunction = listen as ListenFunction,
  emitFunction = emit as EmitFunction
}: {
  invokeFunction?: InvokeFunction;
  listenFunction?: ListenFunction;
  emitFunction?: EmitFunction;
} = {}): ClipboardClient {
  const imageBlob = (value: unknown) => {
    if (value instanceof ArrayBuffer) {
      return new Blob([value], { type: "image/png" });
    }
    if (value instanceof Uint8Array) {
      return new Blob([Uint8Array.from(value)], { type: "image/png" });
    }
    throw new Error("Invalid clipboard image payload");
  };

  return {
    async getHistory(query) {
      return parseClipboardHistoryResult(
        await invokeFunction("get_clipboard_history", { query })
      );
    },

    async getImage(id) {
      return imageBlob(
        await invokeFunction("get_clipboard_history_image", { input: { id } })
      );
    },

    async getSourceIcon(id) {
      return imageBlob(
        await invokeFunction("get_clipboard_history_source_icon", { input: { id } })
      );
    },

    async copyItem(id) {
      return parseClipboardItemActionResult(
        await invokeFunction("copy_clipboard_history_item", { input: { id } }),
        "copied"
      );
    },

    async inputItem(id) {
      return parseClipboardItemActionResult(
        await invokeFunction("input_clipboard_history_item", { input: { id } }),
        "input"
      );
    },

    async closeSurface() {
      return parseClipboardSurfaceCloseResult(
        await invokeFunction("close_clipboard_surface")
      );
    },

    async openPreviewSurface(recordId) {
      await invokeFunction("open_clipboard_preview_surface", { recordId });
    },

    async closePreviewSurface() {
      await invokeFunction("close_clipboard_preview_surface");
    },

    async getPreviewSurfaceState() {
      return parseClipboardPreviewSurfaceState(
        await invokeFunction("get_clipboard_preview_surface_state")
      );
    },

    async subscribePreviewSurface(listener) {
      return listenFunction("clipboard://preview-changed", (event) => {
        listener(parseClipboardPreviewSurfaceChange(event.payload));
      });
    },

    publishPreviewHover(change) {
      return emitFunction("clipboard://preview-hover-changed", change);
    },

    async subscribePreviewHover(listener) {
      return listenFunction("clipboard://preview-hover-changed", (event) => {
        listener(parseClipboardPreviewHoverChange(event.payload));
      });
    },

    async tracePreviewDebug(event, recordId = null, detail = null) {
      await invokeFunction("trace_clipboard_preview_debug", { event, recordId, detail });
    },

    async setSurfaceUnderlayColor(color) {
      await invokeFunction("set_clipboard_surface_underlay_color", { color });
    },

    async setFavorite(id, isFavorite) {
      return parseClipboardHistoryItem(
        await invokeFunction("set_clipboard_history_favorite", {
          input: { id, isFavorite }
        })
      );
    },

    async updateText(id, textContent, expectedRevision) {
      return parseClipboardHistoryItem(
        await invokeFunction("update_clipboard_history_text", {
          input: { id, textContent, expectedRevision }
        })
      );
    },

    async deleteItem(id) {
      return parseClipboardDeleteResult(
        await invokeFunction("delete_clipboard_history_item", { input: { id } })
      );
    },

    async clearUnfavoriteHistory() {
      return parseClipboardClearResult(
        await invokeFunction("clear_unfavorite_clipboard_history")
      );
    },

    subscribe(listener) {
      return listenFunction("clipboard://history-changed", () => listener());
    }
  };
}

export const clipboardClient = createClipboardClient();
