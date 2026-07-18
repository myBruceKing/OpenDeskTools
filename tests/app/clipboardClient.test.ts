import { describe, expect, it, vi } from "vitest";
import { createClipboardClient } from "../../src/app/clipboardClient";

const item = {
  id: "1",
  kind: "text",
  textContent: "真实文本",
  sourceApplication: null,
  sourceProcess: null,
  capturedAtMs: 1_720_000_000_000,
  byteSize: 12,
  isFavorite: false
};

describe("clipboardClient", () => {
  it("maps the frozen command and argument shapes", async () => {
    const invokeFunction = vi.fn(async (command: string) => {
      if (command === "get_clipboard_history") {
        return { items: [item], totalCount: 1 };
      }
      if (command === "set_clipboard_history_favorite") {
        return { ...item, isFavorite: true };
      }
      if (command === "delete_clipboard_history_item") {
        return { deleted: true };
      }
      return { deletedCount: 1 };
    });
    const client = createClipboardClient({ invokeFunction });

    await expect(client.getHistory({ scope: "all", search: null, limit: 100 })).resolves.toEqual({
      items: [item],
      totalCount: 1
    });
    await expect(client.setFavorite("1", true)).resolves.toEqual({ ...item, isFavorite: true });
    await expect(client.deleteItem("1")).resolves.toEqual({ deleted: true });
    await expect(client.clearUnfavoriteHistory()).resolves.toEqual({ deletedCount: 1 });

    expect(invokeFunction).toHaveBeenNthCalledWith(1, "get_clipboard_history", {
      query: { scope: "all", search: null, limit: 100 }
    });
    expect(invokeFunction).toHaveBeenNthCalledWith(2, "set_clipboard_history_favorite", {
      input: { id: "1", isFavorite: true }
    });
    expect(invokeFunction).toHaveBeenNthCalledWith(3, "delete_clipboard_history_item", {
      input: { id: "1" }
    });
    expect(invokeFunction).toHaveBeenNthCalledWith(4, "clear_unfavorite_clipboard_history");
  });

  it("rejects malformed payloads instead of inventing content", async () => {
    const client = createClipboardClient({
      invokeFunction: async () => ({ items: [{ ...item, sourceApplication: undefined }], totalCount: 1 })
    });
    await expect(client.getHistory({ scope: "all" })).rejects.toThrow("sourceApplication");
  });
});
