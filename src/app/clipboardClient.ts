import { invoke } from "@tauri-apps/api/core";
import {
  parseClipboardClearResult,
  parseClipboardDeleteResult,
  parseClipboardHistoryItem,
  parseClipboardHistoryResult,
  type ClipboardHistoryItem,
  type ClipboardHistoryQuery,
  type ClipboardHistoryResult
} from "./clipboardModel";

type InvokeFunction = (command: string, args?: Record<string, unknown>) => Promise<unknown>;

export type ClipboardClient = {
  getHistory: (query: ClipboardHistoryQuery) => Promise<ClipboardHistoryResult>;
  setFavorite: (id: string, isFavorite: boolean) => Promise<ClipboardHistoryItem>;
  deleteItem: (id: string) => Promise<{ deleted: boolean }>;
  clearUnfavoriteHistory: () => Promise<{ deletedCount: number }>;
};

export function createClipboardClient({
  invokeFunction = invoke as InvokeFunction
}: {
  invokeFunction?: InvokeFunction;
} = {}): ClipboardClient {
  return {
    async getHistory(query) {
      return parseClipboardHistoryResult(
        await invokeFunction("get_clipboard_history", { query })
      );
    },

    async setFavorite(id, isFavorite) {
      return parseClipboardHistoryItem(
        await invokeFunction("set_clipboard_history_favorite", {
          input: { id, isFavorite }
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
    }
  };
}

export const clipboardClient = createClipboardClient();
