import { describe, expect, it, vi } from "vitest";
import { createThemeClient } from "../../src/app/themeClient";

const snapshot = {
  mode: "system",
  accent: "#216bd9",
  animationSpeed: "normal",
  reduceTransparency: false,
  background: null,
  backgroundFit: "cover",
  backgroundDim: 24,
  backgroundBlur: 6,
  panelOpacity: 86,
  revision: 3
};

describe("themeClient", () => {
  it("maps get/update commands and the frozen invoke argument shape", async () => {
    const invokeFunction = vi.fn(async (command: string) => {
      if (command === "get_theme_preferences") {
        return snapshot;
      }
      return {
        ...snapshot,
        mode: "dark",
        revision: 4,
        broadcastWarning: { code: "theme_broadcast_failed", message: "同步失败" }
      };
    });
    const client = createThemeClient({ invokeFunction });

    await expect(client.get()).resolves.toEqual(snapshot);
    await expect(client.update(3, { mode: "dark" })).resolves.toEqual({
      snapshot: { ...snapshot, mode: "dark", revision: 4 },
      broadcastWarning: { code: "theme_broadcast_failed", message: "同步失败" }
    });
    expect(invokeFunction).toHaveBeenNthCalledWith(1, "get_theme_preferences");
    expect(invokeFunction).toHaveBeenNthCalledWith(2, "update_theme_preferences", {
      patch: { expectedRevision: 3, mode: "dark" }
    });
  });

  it("maps the change event and returns its cleanup function", async () => {
    let eventHandler: ((event: { payload: unknown }) => void) | undefined;
    const cleanup = vi.fn();
    const listenFunction = vi.fn(async (_event, handler) => {
      eventHandler = handler;
      return cleanup;
    });
    const listener = vi.fn();
    const client = createThemeClient({ listenFunction });

    const unlisten = await client.subscribe(listener);
    eventHandler?.({ payload: snapshot });

    expect(listenFunction).toHaveBeenCalledWith("theme://changed", expect.any(Function));
    expect(listener).toHaveBeenCalledWith(snapshot);
    unlisten();
    expect(cleanup).toHaveBeenCalledOnce();
  });

  it("maps background selection, removal, cancellation and raw PNG reads", async () => {
    const bytes = new Uint8Array([137, 80, 78, 71]);
    const invokeFunction = vi.fn(async (command: string) => {
      if (command === "select_theme_background") {
        return null;
      }
      if (command === "remove_theme_background") {
        return { ...snapshot, revision: 4, broadcastWarning: null };
      }
      if (command === "get_theme_background_image") {
        return bytes;
      }
      return snapshot;
    });
    const client = createThemeClient({ invokeFunction });

    await expect(client.selectBackground(3)).resolves.toBeNull();
    await expect(client.removeBackground(3)).resolves.toEqual({
      snapshot: { ...snapshot, revision: 4 },
      broadcastWarning: null
    });
    const blob = await client.getBackgroundImage();
    expect(blob.type).toBe("image/png");
    expect(blob.size).toBe(4);
    expect(invokeFunction).toHaveBeenNthCalledWith(1, "select_theme_background", {
      input: { expectedRevision: 3 }
    });
    expect(invokeFunction).toHaveBeenNthCalledWith(2, "remove_theme_background", {
      input: { expectedRevision: 3 }
    });
    expect(invokeFunction).toHaveBeenNthCalledWith(3, "get_theme_background_image");
  });

  it("accepts canonical custom accents and rejects malformed colors", async () => {
    const customClient = createThemeClient({
      invokeFunction: async () => ({ ...snapshot, accent: "#f4e04d" })
    });
    await expect(customClient.get()).resolves.toMatchObject({ accent: "#f4e04d" });

    const client = createThemeClient({
      invokeFunction: async () => ({ ...snapshot, accent: "#fffffg" })
    });

    await expect(client.get()).rejects.toThrow("accent");
  });

  it("accepts the full background dim range and rejects values beyond 100 percent", async () => {
    const fullDimClient = createThemeClient({
      invokeFunction: async () => ({ ...snapshot, backgroundDim: 100 })
    });
    await expect(fullDimClient.get()).resolves.toMatchObject({ backgroundDim: 100 });

    const invalidDimClient = createThemeClient({
      invokeFunction: async () => ({ ...snapshot, backgroundDim: 101 })
    });
    await expect(invalidDimClient.get()).rejects.toThrow("backgroundDim");
  });

  it("accepts the full panel opacity range", async () => {
    const transparentPanelClient = createThemeClient({
      invokeFunction: async () => ({ ...snapshot, panelOpacity: 0 })
    });
    await expect(transparentPanelClient.get()).resolves.toMatchObject({ panelOpacity: 0 });

    const invalidPanelClient = createThemeClient({
      invokeFunction: async () => ({ ...snapshot, panelOpacity: 101 })
    });
    await expect(invalidPanelClient.get()).rejects.toThrow("panelOpacity");
  });
});
