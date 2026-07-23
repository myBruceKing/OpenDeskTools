import { describe, expect, it, vi } from "vitest";
import type { ThemeClient } from "../../src/app/themeClient";
import { ThemeController } from "../../src/app/themeController";
import type { ThemeSnapshot } from "../../src/app/themeModel";

function makeSnapshot(revision = 0, overrides: Partial<ThemeSnapshot> = {}): ThemeSnapshot {
  const snapshot: ThemeSnapshot = {
    mode: "system",
    accent: "#216bd9",
    animationSpeed: "normal",
    reduceTransparency: false,
    background: null,
    backgroundFit: "cover",
    backgroundDim: 24,
    backgroundBlur: 6,
    panelOpacity: 86,
    revision,
  };
  return { ...snapshot, ...overrides, background: overrides.background ?? snapshot.background };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

async function flush() {
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
}

function makeClient(overrides: Partial<ThemeClient> = {}): ThemeClient {
  return {
    get: async () => makeSnapshot(),
    update: async (_expectedRevision, patch) => ({
      snapshot: makeSnapshot(1, patch),
      broadcastWarning: null
    }),
    selectBackground: async () => null,
    removeBackground: async (expectedRevision) => ({
      snapshot: makeSnapshot(expectedRevision + 1, { background: null }),
      broadcastWarning: null
    }),
    getBackgroundImage: async () => new Blob(),
    subscribe: async () => () => undefined,
    ...overrides
  };
}

describe("ThemeController", () => {
  it("loads a confirmed snapshot and exposes failures truthfully", async () => {
    const readyController = new ThemeController(makeClient());
    readyController.start();
    await flush();
    expect(readyController.getSnapshot()).toMatchObject({
      status: "ready",
      current: makeSnapshot()
    });

    const failedController = new ThemeController(
      makeClient({
        get: async () => Promise.reject({
          code: "theme_unavailable",
          message: "不可用",
          field: null,
          retryable: true,
          applied: false
        })
      })
    );
    failedController.start();
    await flush();
    expect(failedController.getSnapshot()).toMatchObject({
      status: "unavailable",
      current: null,
      error: { code: "theme_unavailable", message: "不可用" }
    });
  });

  it("ignores duplicate/stale events and cleans up the subscription", async () => {
    let eventListener: ((snapshot: ThemeSnapshot) => void) | undefined;
    const unlisten = vi.fn();
    const controller = new ThemeController(
      makeClient({
        get: async () => makeSnapshot(2),
        subscribe: async (listener) => {
          eventListener = listener;
          return unlisten;
        }
      })
    );

    controller.start();
    await flush();
    eventListener?.(makeSnapshot(1, { mode: "dark" }));
    eventListener?.(makeSnapshot(2, { mode: "dark" }));
    expect(controller.getSnapshot().current).toEqual(makeSnapshot(2));
    eventListener?.(makeSnapshot(3, { mode: "dark" }));
    expect(controller.getSnapshot().current).toEqual(makeSnapshot(3, { mode: "dark" }));

    controller.stop();
    expect(unlisten).toHaveBeenCalledOnce();
  });

  it("keeps a newer event snapshot when the concurrent initial get fails", async () => {
    const request = deferred<ThemeSnapshot>();
    let eventListener: ((snapshot: ThemeSnapshot) => void) | undefined;
    const controller = new ThemeController(
      makeClient({
        get: () => request.promise,
        subscribe: async (listener) => {
          eventListener = listener;
          return () => undefined;
        }
      })
    );

    controller.start();
    await flush();
    eventListener?.(makeSnapshot(3, { mode: "dark" }));
    request.reject({
      code: "theme_unavailable",
      message: "读取失败",
      field: null,
      retryable: true,
      applied: false
    });
    await flush();

    expect(controller.getSnapshot()).toMatchObject({
      status: "ready",
      current: makeSnapshot(3, { mode: "dark" }),
      error: null,
      warning: { code: "theme_unavailable" }
    });
  });

  it("serializes optimistic partial saves and advances expectedRevision", async () => {
    const firstUpdate = deferred<{ snapshot: ThemeSnapshot; broadcastWarning: null }>();
    const update = vi
      .fn<ThemeClient["update"]>()
      .mockImplementationOnce(() => firstUpdate.promise)
      .mockResolvedValueOnce({
        snapshot: makeSnapshot(2, { mode: "dark", animationSpeed: "fast" }),
        broadcastWarning: null
      });
    const controller = new ThemeController(makeClient({ update }));
    controller.start();
    await flush();

    const first = controller.update({ mode: "dark" });
    const second = controller.update({ animationSpeed: "fast" });
    expect(controller.getSnapshot().current).toMatchObject({ mode: "dark", animationSpeed: "fast" });
    expect(update).toHaveBeenCalledTimes(1);
    expect(update).toHaveBeenNthCalledWith(1, 0, { mode: "dark" });

    firstUpdate.resolve({
      snapshot: makeSnapshot(1, { mode: "dark" }),
      broadcastWarning: null
    });
    await first;
    await flush();
    expect(update).toHaveBeenNthCalledWith(2, 1, { animationSpeed: "fast" });
    await second;
    expect(controller.getSnapshot()).toMatchObject({
      saving: false,
      current: makeSnapshot(2, { mode: "dark", animationSpeed: "fast" })
    });
  });

  it("rolls back applied=false and refreshes a revision conflict before the next save", async () => {
    const update = vi
      .fn<ThemeClient["update"]>()
      .mockRejectedValueOnce({
        code: "theme_revision_conflict",
        message: "设置已在其他窗口修改",
        field: "expectedRevision",
        retryable: true,
        applied: false
      })
      .mockResolvedValueOnce({
        snapshot: makeSnapshot(5, { accent: "#7955c7", mode: "dark" }),
        broadcastWarning: null
      });
    const get = vi
      .fn<ThemeClient["get"]>()
      .mockResolvedValueOnce(makeSnapshot(0))
      .mockResolvedValueOnce(makeSnapshot(4, { accent: "#7955c7" }));
    const controller = new ThemeController(makeClient({ get, update }));
    controller.start();
    await flush();

    await controller.update({ mode: "dark" });
    expect(controller.getSnapshot()).toMatchObject({
      current: makeSnapshot(4, { accent: "#7955c7" }),
      error: { code: "theme_revision_conflict", applied: false }
    });

    await controller.update({ mode: "dark" });
    expect(update).toHaveBeenNthCalledWith(2, 4, { mode: "dark" });
    expect(controller.getSnapshot().current).toEqual(
      makeSnapshot(5, { accent: "#7955c7", mode: "dark" })
    );
  });

  it("reports the real refresh failure and abandons queued writes after conflict recovery fails", async () => {
    const update = vi.fn<ThemeClient["update"]>().mockRejectedValue({
      code: "theme_revision_conflict",
      message: "revision conflict",
      field: "expectedRevision",
      retryable: true,
      applied: false
    });
    const refresh = deferred<ThemeSnapshot>();
    const get = vi
      .fn<ThemeClient["get"]>()
      .mockResolvedValueOnce(makeSnapshot(0))
      .mockImplementationOnce(() => refresh.promise);
    const controller = new ThemeController(makeClient({ get, update }));
    controller.start();
    await flush();

    const first = controller.update({ mode: "dark" });
    const queued = controller.update({ animationSpeed: "fast" });
    refresh.reject({
      code: "theme_unavailable",
      message: "refresh failed",
      field: null,
      retryable: true,
      applied: false
    });
    await Promise.all([first, queued]);

    expect(update).toHaveBeenCalledTimes(1);
    expect(controller.getSnapshot()).toMatchObject({
      status: "unavailable",
      current: null,
      saving: false,
      error: { code: "theme_unavailable", message: "refresh failed" }
    });
  });

  it("keeps a successful update when broadcasting warns", async () => {
    const controller = new ThemeController(
      makeClient({
        update: async () => ({
          snapshot: makeSnapshot(1, { reduceTransparency: true }),
          broadcastWarning: { code: "theme_broadcast_failed", message: "其他窗口未同步" }
        })
      })
    );
    controller.start();
    await flush();

    await controller.update({ reduceTransparency: true });
    expect(controller.getSnapshot()).toMatchObject({
      current: makeSnapshot(1, { reduceTransparency: true }),
      error: null,
      warning: { code: "theme_broadcast_failed" }
    });
  });

  it("applies and removes a selected background through revision-checked mutations", async () => {
    const background = {
      id: "a".repeat(64),
      fileName: "forest.webp",
      byteSize: 2048,
      width: 1920,
      height: 1080
    };
    const selectBackground = vi.fn<ThemeClient["selectBackground"]>().mockResolvedValue({
      snapshot: makeSnapshot(1, { background }),
      broadcastWarning: null
    });
    const removeBackground = vi.fn<ThemeClient["removeBackground"]>().mockResolvedValue({
      snapshot: makeSnapshot(2, { background: null }),
      broadcastWarning: null
    });
    const controller = new ThemeController(makeClient({ selectBackground, removeBackground }));
    controller.start();
    await flush();

    await controller.selectBackground();
    expect(selectBackground).toHaveBeenCalledWith(0);
    expect(controller.getSnapshot().current?.background).toEqual(background);

    await controller.removeBackground();
    expect(removeBackground).toHaveBeenCalledWith(1);
    expect(controller.getSnapshot().current?.background).toBeNull();
    expect(controller.getSnapshot().saving).toBe(false);
  });

  it("keeps the current theme unchanged when the native image picker is cancelled", async () => {
    const selectBackground = vi.fn<ThemeClient["selectBackground"]>().mockResolvedValue(null);
    const controller = new ThemeController(makeClient({ selectBackground }));
    controller.start();
    await flush();

    await controller.selectBackground();

    expect(controller.getSnapshot()).toMatchObject({
      current: makeSnapshot(),
      saving: false,
      error: null
    });
  });

  it("does not apply a late initial response after stop", async () => {
    const request = deferred<ThemeSnapshot>();
    const controller = new ThemeController(makeClient({ get: () => request.promise }));
    controller.start();
    const stoppedState = controller.getSnapshot();
    controller.stop();

    request.resolve(makeSnapshot(8, { mode: "dark" }));
    await flush();
    expect(controller.getSnapshot()).toBe(stoppedState);
  });

  it("does not apply a late update response after stop", async () => {
    const request = deferred<{ snapshot: ThemeSnapshot; broadcastWarning: null }>();
    const controller = new ThemeController(
      makeClient({ update: () => request.promise })
    );
    controller.start();
    await flush();
    const updatePromise = controller.update({ mode: "dark" });
    controller.stop();
    const stoppedState = controller.getSnapshot();

    request.resolve({ snapshot: makeSnapshot(1, { mode: "dark" }), broadcastWarning: null });
    await updatePromise;
    await flush();
    expect(controller.getSnapshot()).toBe(stoppedState);
  });

  it("does not enqueue or call the client when update is requested after stop", async () => {
    const update = vi.fn<ThemeClient["update"]>();
    const controller = new ThemeController(makeClient({ update }));
    controller.start();
    await flush();
    controller.stop();
    const stoppedState = controller.getSnapshot();

    await controller.update({ mode: "dark" });

    expect(update).not.toHaveBeenCalled();
    expect(controller.getSnapshot()).toBe(stoppedState);
  });
});
