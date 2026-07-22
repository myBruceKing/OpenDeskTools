import { describe, expect, it, vi } from "vitest";
import { createClipboardClient } from "../../src/app/clipboardClient";

const item = {
  id: "1",
  revision: 1,
  kind: "text",
  textContent: "真实文本",
  sourceApplication: null,
  sourceProcess: null,
  capturedAtMs: 1_720_000_000_000,
  byteSize: 12,
  isFavorite: false,
  sourceIconAvailable: true,
  fileCount: null,
  fileNames: null,
  displayCategory: "text"
};

describe("clipboardClient", () => {
  it("maps the frozen command and argument shapes", async () => {
    const invokeFunction = vi.fn(async (command: string) => {
      if (command === "get_clipboard_history") {
        return {
          items: [item], totalCount: 1, monitoring: "running", surfaceActive: true, inputAvailable: true
        };
      }
      if (command === "set_clipboard_monitoring") {
        return "paused";
      }
      if (command === "copy_clipboard_history_item") {
        return { action: "copied", clipboardUpdated: true };
      }
      if (command === "input_clipboard_history_item") {
        return { action: "input", clipboardUpdated: true };
      }
      if (command === "close_clipboard_surface") {
        return { closed: true, inputAvailable: false };
      }
      if (command === "get_clipboard_preview_surface_state") {
        return { recordId: "1", visible: true };
      }
      if (command === "set_clipboard_history_favorite") {
        return { ...item, isFavorite: true };
      }
      if (command === "update_clipboard_history_text") {
        return { ...item, revision: 2, textContent: "已编辑" };
      }
      if (command === "delete_clipboard_history_item") {
        return { deleted: true };
      }
      if (command === "get_clipboard_history_image" || command === "get_clipboard_history_source_icon") {
        return new Uint8Array([137, 80, 78, 71]);
      }
      return { deletedCount: 1 };
    });
    const client = createClipboardClient({ invokeFunction });

    await expect(client.getHistory({ scope: "all", search: null, limit: 100 })).resolves.toEqual({
      items: [item],
      totalCount: 1,
      monitoring: "running",
      surfaceActive: true,
      inputAvailable: true
    });
    await expect(client.setMonitoring(false)).resolves.toBe("paused");
    const image = await client.getImage("1");
    expect(image.type).toBe("image/png");
    expect(Array.from(new Uint8Array(await image.arrayBuffer()))).toEqual([137, 80, 78, 71]);
    await expect(client.getSourceIcon("1")).resolves.toMatchObject({ type: "image/png", size: 4 });
    await expect(client.copyItem("1")).resolves.toEqual({ action: "copied", clipboardUpdated: true });
    await expect(client.inputItem("1")).resolves.toEqual({ action: "input", clipboardUpdated: true });
    await expect(client.closeSurface()).resolves.toEqual({ closed: true, inputAvailable: false });
    await expect(client.setFavorite("1", true)).resolves.toEqual({ ...item, isFavorite: true });
    await expect(client.updateText("1", "已编辑", 1)).resolves.toEqual({ ...item, revision: 2, textContent: "已编辑" });
    await expect(client.deleteItem("1")).resolves.toEqual({ deleted: true });
    await expect(client.clearUnfavoriteHistory()).resolves.toEqual({ deletedCount: 1 });
    await expect(client.openPreviewSurface("1")).resolves.toBeUndefined();
    await expect(client.closePreviewSurface()).resolves.toBeUndefined();
    await expect(client.getPreviewSurfaceState()).resolves.toEqual({ recordId: "1", visible: true });
    await expect(client.tracePreviewDebug("close_scheduled", "1")).resolves.toBeUndefined();
    await expect(client.setSurfaceUnderlayColor("#E0DEDC")).resolves.toBeUndefined();

    expect(invokeFunction).toHaveBeenNthCalledWith(1, "get_clipboard_history", {
      query: { scope: "all", search: null, limit: 100 }
    });
    expect(invokeFunction).toHaveBeenCalledWith("get_clipboard_history_image", {
      input: { id: "1" }
    });
    expect(invokeFunction).toHaveBeenNthCalledWith(4, "get_clipboard_history_source_icon", {
      input: { id: "1" }
    });
    expect(invokeFunction).toHaveBeenNthCalledWith(5, "copy_clipboard_history_item", {
      input: { id: "1" }
    });
    expect(invokeFunction).toHaveBeenNthCalledWith(6, "input_clipboard_history_item", {
      input: { id: "1" }
    });
    expect(invokeFunction).toHaveBeenNthCalledWith(7, "close_clipboard_surface");
    expect(invokeFunction).toHaveBeenNthCalledWith(8, "set_clipboard_history_favorite", {
      input: { id: "1", isFavorite: true }
    });
    expect(invokeFunction).toHaveBeenNthCalledWith(9, "update_clipboard_history_text", {
      input: { id: "1", textContent: "已编辑", expectedRevision: 1 }
    });
    expect(invokeFunction).toHaveBeenNthCalledWith(10, "delete_clipboard_history_item", {
      input: { id: "1" }
    });
    expect(invokeFunction).toHaveBeenNthCalledWith(11, "clear_unfavorite_clipboard_history");
    expect(invokeFunction).toHaveBeenNthCalledWith(12, "open_clipboard_preview_surface", { recordId: "1" });
    expect(invokeFunction).toHaveBeenNthCalledWith(13, "close_clipboard_preview_surface");
    expect(invokeFunction).toHaveBeenNthCalledWith(14, "get_clipboard_preview_surface_state");
    expect(invokeFunction).toHaveBeenNthCalledWith(15, "trace_clipboard_preview_debug", {
      event: "close_scheduled",
      recordId: "1",
      detail: null
    });
    expect(invokeFunction).toHaveBeenNthCalledWith(16, "set_clipboard_surface_underlay_color", {
      color: "#E0DEDC"
    });
  });

  it("accepts raw ArrayBuffer images and rejects JSON-like image payloads", async () => {
    const bytes = new Uint8Array([1, 2, 3]).buffer;
    const rawClient = createClipboardClient({ invokeFunction: async () => bytes });
    await expect(rawClient.getImage("7")).resolves.toMatchObject({ size: 3, type: "image/png" });

    const jsonClient = createClipboardClient({ invokeFunction: async () => [1, 2, 3] });
    await expect(jsonClient.getImage("7")).rejects.toThrow("Invalid clipboard image payload");
  });

  it("rejects malformed action and surface results", async () => {
    const malformedCopy = createClipboardClient({
      invokeFunction: async () => ({ action: "copied", clipboardUpdated: false })
    });
    await expect(malformedCopy.copyItem("1")).rejects.toThrow("item action payload");

    const malformedInput = createClipboardClient({
      invokeFunction: async () => ({ action: "copied", clipboardUpdated: true })
    });
    await expect(malformedInput.inputItem("1")).rejects.toThrow("item action payload");

    const malformedClose = createClipboardClient({
      invokeFunction: async () => ({ closed: true, inputAvailable: true })
    });
    await expect(malformedClose.closeSurface()).rejects.toThrow("surface close");
  });

  it("rejects malformed payloads instead of inventing content", async () => {
    const client = createClipboardClient({
      invokeFunction: async () => ({
        items: [{ ...item, sourceApplication: undefined }],
        totalCount: 1,
        monitoring: "running",
        surfaceActive: false,
        inputAvailable: false
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

  it("maps preview lifecycle and cross-window hover events without clipboard content", async () => {
    const handlers = new Map<string, (event: { payload: unknown }) => void>();
    const unlisten = vi.fn();
    const listenFunction = vi.fn(async (event: string, handler: (event: { payload: unknown }) => void) => {
      handlers.set(event, handler);
      return unlisten;
    });
    const emitFunction = vi.fn(async () => undefined);
    const client = createClipboardClient({ listenFunction, emitFunction });
    const previewListener = vi.fn();
    const hoverListener = vi.fn();

    const stopPreview = await client.subscribePreviewSurface(previewListener);
    const stopHover = await client.subscribePreviewHover(hoverListener);
    handlers.get("clipboard://preview-changed")?.({
      payload: { change: "selection_changed", recordId: "7", visible: true }
    });
    handlers.get("clipboard://preview-hover-changed")?.({
      payload: { inside: true, recordId: "7" }
    });
    await client.publishPreviewHover({ inside: false, recordId: "7" });

    expect(previewListener).toHaveBeenCalledWith({
      change: "selection_changed", recordId: "7", visible: true
    });
    expect(hoverListener).toHaveBeenCalledWith({ inside: true, recordId: "7" });
    expect(emitFunction).toHaveBeenCalledWith(
      "clipboard://preview-hover-changed",
      { inside: false, recordId: "7" }
    );
    stopPreview();
    stopHover();
    expect(unlisten).toHaveBeenCalledTimes(2);
  });
});
