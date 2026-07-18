import { describe, expect, it, vi } from "vitest";
import { createThemeClient } from "../../src/app/themeClient";

const snapshot = {
  mode: "system",
  accent: "#216bd9",
  animationSpeed: "normal",
  reduceTransparency: false,
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

  it("rejects snapshots outside the six-color allowlist", async () => {
    const client = createThemeClient({
      invokeFunction: async () => ({ ...snapshot, accent: "#ffffff" })
    });

    await expect(client.get()).rejects.toThrow("accent");
  });
});
