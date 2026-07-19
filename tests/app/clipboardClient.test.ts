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
        return { items: [item], totalCount: 1, monitoring: "running" };
      }
      if (command === "set_clipboard_history_favorite") {
        return { ...item, isFavorite: true };
      }
      if (command === "delete_clipboard_history_item") {
        return { deleted: true };
      }
      if (command === "get_clipboard_history_image") {
        return new Uint8Array([137, 80, 78, 71]);
      }
      return { deletedCount: 1 };
    });
    const client = createClipboardClient({ invokeFunction });

    await expect(client.getHistory({ scope: "all", search: null, limit: 100 })).resolves.toEqual({
      items: [item],
      totalCount: 1,
      monitoring: "running"
    });
    const image = await client.getImage("1");
    expect(image.type).toBe("image/png");
    expect(Array.from(new Uint8Array(await image.arrayBuffer()))).toEqual([137, 80, 78, 71]);
    await expect(client.setFavorite("1", true)).resolves.toEqual({ ...item, isFavorite: true });
    await expect(client.deleteItem("1")).resolves.toEqual({ deleted: true });
    await expect(client.clearUnfavoriteHistory()).resolves.toEqual({ deletedCount: 1 });

    expect(invokeFunction).toHaveBeenNthCalledWith(1, "get_clipboard_history", {
      query: { scope: "all", search: null, limit: 100 }
    });
    expect(invokeFunction).toHaveBeenNthCalledWith(2, "get_clipboard_history_image", {
      input: { id: "1" }
    });
    expect(invokeFunction).toHaveBeenNthCalledWith(3, "set_clipboard_history_favorite", {
      input: { id: "1", isFavorite: true }
    });
    expect(invokeFunction).toHaveBeenNthCalledWith(4, "delete_clipboard_history_item", {
      input: { id: "1" }
    });
    expect(invokeFunction).toHaveBeenNthCalledWith(5, "clear_unfavorite_clipboard_history");
  });

  it("accepts raw ArrayBuffer images and rejects JSON-like image payloads", async () => {
    const bytes = new Uint8Array([1, 2, 3]).buffer;
    const rawClient = createClipboardClient({ invokeFunction: async () => bytes });
    await expect(rawClient.getImage("7")).resolves.toMatchObject({ size: 3, type: "image/png" });

    const jsonClient = createClipboardClient({ invokeFunction: async () => [1, 2, 3] });
    await expect(jsonClient.getImage("7")).rejects.toThrow("Invalid clipboard image payload");
  });

  it("rejects malformed payloads instead of inventing content", async () => {
    const client = createClipboardClient({
      invokeFunction: async () => ({
        items: [{ ...item, sourceApplication: undefined }],
        totalCount: 1,
        monitoring: "running"
      })
    });
    await expect(client.getHistory({ scope: "all" })).rejects.toThrow("sourceApplication");
  });

  it("subscribes to the frozen history event and returns the real unlisten function", async () => {
    let handler: ((event: { payload: unknown }) => void) | undefined;
    const unlisten = vi.fn();
    const listenFunction = vi.fn(async (_event, eventHandler) => {
      handler = eventHandler;
      return unlisten;
    });
    const listener = vi.fn();
    const client = createClipboardClient({ listenFunction });

    const cleanup = await client.subscribe(listener);
    expect(listenFunction).toHaveBeenCalledWith(
      "clipboard://history-changed",
      expect.any(Function)
    );
    handler?.({ payload: { ignored: "payload carries no clipboard content" } });
    expect(listener).toHaveBeenCalledOnce();
    cleanup();
    expect(unlisten).toHaveBeenCalledOnce();
  });
});
