import { describe, expect, it, vi } from "vitest";
import type { ClipboardClient } from "../../src/app/clipboardClient";
import { ClipboardController } from "../../src/app/clipboardController";
import type { ClipboardHistoryItem, ClipboardHistoryResult } from "../../src/app/clipboardModel";

const first: ClipboardHistoryItem = {
  id: "1",
  kind: "text",
  textContent: "第一条",
  sourceApplication: null,
  sourceProcess: null,
  capturedAtMs: 1_720_000_000_000,
  byteSize: 9,
  isFavorite: false
};

const favorite: ClipboardHistoryItem = {
  ...first,
  id: "2",
  textContent: "已收藏",
  isFavorite: true
};

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
}

function makeClient(overrides: Partial<ClipboardClient> = {}): ClipboardClient {
  return {
    getHistory: async () => ({ items: [first, favorite], totalCount: 2 }),
    setFavorite: async (id, isFavorite) => ({
      ...(id === favorite.id ? favorite : first),
      isFavorite
    }),
    deleteItem: async () => ({ deleted: true }),
    clearUnfavoriteHistory: async () => ({ deletedCount: 1 }),
    ...overrides
  };
}

describe("ClipboardController", () => {
  it("loads at most 100 real records and exposes the first-slice availability", async () => {
    const getHistory = vi.fn<ClipboardClient["getHistory"]>(async () => ({ items: [first], totalCount: 1 }));
    const controller = new ClipboardController(makeClient({ getHistory }));
    controller.start();
    expect(controller.getSnapshot()).toMatchObject({
      status: "loading",
      viewModel: { monitoring: "paused" }
    });
    await flush();

    expect(getHistory).toHaveBeenCalledWith({ scope: "all", search: null, limit: 100 });
    expect(controller.getSnapshot()).toMatchObject({
      status: "ready",
      viewModel: {
        monitoring: "paused",
        totalCount: 1,
        settings: { maxItems: "100", duplicateStrategy: "相同内容移到最前" },
        actions: {
          canCopy: false,
          canTypeIntoTarget: false,
          canFavorite: true,
          canDelete: true,
          canOpenSource: false,
          canClearHistory: true
        }
      }
    });
  });

  it("marks IPC load failures unavailable with a safe mapped message", async () => {
    const controller = new ClipboardController(makeClient({
      getHistory: async () => Promise.reject({
        code: "clipboard_history_unavailable",
        message: "SQLITE: database C:\\private\\history.db is locked",
        retryable: true
      })
    }));
    controller.start();
    await flush();

    expect(controller.getSnapshot()).toMatchObject({
      status: "unavailable",
      viewModel: { monitoring: "unavailable", items: [] },
      error: {
        code: "clipboard_history_unavailable",
        message: "剪贴板历史服务暂时不可用，请稍后重试。",
        retryable: true
      }
    });
  });

  it("ignores a late load after stop", async () => {
    const request = deferred<ClipboardHistoryResult>();
    const controller = new ClipboardController(makeClient({ getHistory: () => request.promise }));
    controller.start();
    const loadingState = controller.getSnapshot();
    controller.stop();
    request.resolve({ items: [first], totalCount: 1 });
    await flush();
    expect(controller.getSnapshot()).toBe(loadingState);
  });

  it("applies confirmed favorite/delete/clear results without local-only mutations", async () => {
    const favoriteRequest = deferred<ClipboardHistoryItem>();
    const controller = new ClipboardController(makeClient({
      setFavorite: () => favoriteRequest.promise
    }));
    controller.start();
    await flush();

    const favoritePromise = controller.setFavorite(first.id, true);
    expect(controller.getSnapshot().viewModel.items[0].favorite).toBe(false);
    expect(controller.getSnapshot().pendingItemIds).toEqual([first.id]);
    favoriteRequest.resolve({ ...first, isFavorite: true });
    await favoritePromise;
    expect(controller.getSnapshot().viewModel.items[0].favorite).toBe(true);

    await controller.deleteItem(first.id);
    expect(controller.getSnapshot().viewModel.items.map((item) => item.id)).toEqual([favorite.id]);
    expect(controller.getSnapshot().viewModel.totalCount).toBe(1);

    await controller.clearUnfavoriteHistory();
    expect(controller.getSnapshot().viewModel.items.map((item) => item.id)).toEqual([favorite.id]);
    expect(controller.getSnapshot().viewModel.totalCount).toBe(1);
  });

  it("keeps the confirmed item and exposes an operation failure for retry", async () => {
    const controller = new ClipboardController(makeClient({
      deleteItem: async () => ({ deleted: false })
    }));
    controller.start();
    await flush();
    await controller.deleteItem(first.id);

    expect(controller.getSnapshot()).toMatchObject({
      status: "ready",
      error: { code: "clipboard_operation_not_applied", retryable: true }
    });
    expect(controller.getSnapshot().viewModel.items.some((item) => item.id === first.id)).toBe(true);
    expect(controller.getSnapshot().pendingItemIds).toEqual([]);
  });

  it("keeps another item's later error when an earlier request succeeds late", async () => {
    const firstRequest = deferred<ClipboardHistoryItem>();
    const secondRequest = deferred<ClipboardHistoryItem>();
    const secondRetry = deferred<ClipboardHistoryItem>();
    let secondCalls = 0;
    const controller = new ClipboardController(makeClient({
      setFavorite: (id) => {
        if (id === first.id) {
          return firstRequest.promise;
        }
        secondCalls += 1;
        return secondCalls === 1 ? secondRequest.promise : secondRetry.promise;
      }
    }));
    controller.start();
    await flush();

    const firstMutation = controller.setFavorite(first.id, true);
    const secondMutation = controller.setFavorite(favorite.id, false);
    secondRequest.reject({
      code: "clipboard_history_unavailable",
      message: "SQL SELECT * FROM clipboard_history",
      retryable: true
    });
    await secondMutation;
    expect(controller.getSnapshot().error?.code).toBe("clipboard_history_unavailable");

    firstRequest.resolve({ ...first, isFavorite: true });
    await firstMutation;
    expect(controller.getSnapshot().error?.code).toBe("clipboard_history_unavailable");
    expect(controller.getSnapshot().error?.message).not.toContain("SQL");

    const retry = controller.setFavorite(favorite.id, false);
    secondRetry.resolve({ ...favorite, isFavorite: false });
    await retry;
    expect(controller.getSnapshot().error).toBeNull();
  });
});
