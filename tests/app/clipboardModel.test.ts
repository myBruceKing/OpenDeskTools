import { describe, expect, it } from "vitest";
import {
  EMPTY_CLIPBOARD_VIEW_MODEL,
  getClipboardMonitoringPresentation,
  normalizeClipboardCommandError,
  parseClipboardHistoryResult,
  toClipboardItemViewModel
} from "../../src/app/clipboardModel";

describe("EMPTY_CLIPBOARD_VIEW_MODEL", () => {
  it("contains no content and exposes no available actions", () => {
    expect(EMPTY_CLIPBOARD_VIEW_MODEL.monitoring).toBe("unavailable");
    expect(EMPTY_CLIPBOARD_VIEW_MODEL.totalCount).toBe(0);
    expect(EMPTY_CLIPBOARD_VIEW_MODEL.items).toEqual([]);
    expect(Object.values(EMPTY_CLIPBOARD_VIEW_MODEL.settings).every((value) => value === null)).toBe(true);
    expect(Object.values(EMPTY_CLIPBOARD_VIEW_MODEL.actions).every((available) => !available)).toBe(true);
  });

  it.each([
    ["running", { label: "监控运行中", checked: true, disabled: true }],
    ["paused", { label: "监控已暂停", checked: false, disabled: true }],
    ["unavailable", { label: "监控不可用", checked: null, disabled: true }]
  ] as const)("maps %s monitoring state without inventing a value", (monitoring, expected) => {
    expect(getClipboardMonitoringPresentation(monitoring)).toEqual(expected);
  });

  it("derives text presentation from raw content without fabricating source or privacy", () => {
    const item = toClipboardItemViewModel({
      id: "1",
      kind: "text",
      textContent: "\n  第一行标题  \n第二行内容",
      sourceApplication: null,
      sourceProcess: null,
      capturedAtMs: 1_720_000_000_000,
      byteSize: 32,
      isFavorite: false
    });

    expect(item).toMatchObject({
      title: "第一行标题",
      preview: "\n  第一行标题  \n第二行内容",
      sourceApp: "来源不可用",
      sourceProcess: "来源不可用",
      privacy: "unknown",
      locked: false,
      iconTone: "note"
    });
    expect(item.capturedAt).not.toBe("");
    expect(item.size).toContain("32 B");
  });

  it("does not invent an image preview when a future image record is received", () => {
    const item = toClipboardItemViewModel({
      id: "2",
      kind: "image",
      textContent: null,
      sourceApplication: null,
      sourceProcess: null,
      capturedAtMs: Number.MAX_SAFE_INTEGER,
      byteSize: 2048,
      isFavorite: true
    });
    expect(item.preview).toBe("图片预览暂不可用");
    expect(item.capturedAt).toBe("时间不可用");
  });

  it("rejects duplicate ids, invalid counts, and text records without text", () => {
    const valid = {
      id: "1",
      kind: "text",
      textContent: "内容",
      sourceApplication: null,
      sourceProcess: null,
      capturedAtMs: 1,
      byteSize: 6,
      isFavorite: false
    };
    expect(() => parseClipboardHistoryResult({ items: [valid, valid], totalCount: 2, monitoring: "running" })).toThrow("duplicate id");
    expect(() => parseClipboardHistoryResult({ items: [valid], totalCount: 0, monitoring: "running" })).toThrow("totalCount");
    expect(() => parseClipboardHistoryResult({
      items: [{ ...valid, textContent: null }],
      totalCount: 1,
      monitoring: "running"
    })).toThrow("textContent");
  });

  it("maps command and parser failures to safe copy without exposing internals", () => {
    expect(normalizeClipboardCommandError({
      code: "clipboard_history_unavailable",
      message: "SQLITE_BUSY at C:\\private\\history.db",
      retryable: false
    })).toEqual({
      code: "clipboard_history_unavailable",
      message: "剪贴板历史服务暂时不可用，请稍后重试。",
      retryable: true
    });
    expect(normalizeClipboardCommandError(new Error("Invalid clipboard payload field: sourceProcess")))
      .toEqual({
        code: "clipboard_client_failed",
        message: "剪贴板历史服务暂时不可用，请稍后重试。",
        retryable: true
      });
    expect(normalizeClipboardCommandError({
      code: "database_schema_secret",
      message: "SELECT token FROM users"
    })).toEqual({
      code: "clipboard_client_failed",
      message: "剪贴板历史服务暂时不可用，请稍后重试。",
      retryable: true
    });
  });

  it.each(["running", "unavailable"] as const)(
    "accepts the backend monitoring truth %s",
    (monitoring) => {
      expect(parseClipboardHistoryResult({ items: [], totalCount: 0, monitoring }).monitoring)
        .toBe(monitoring);
    }
  );

  it.each(["paused", "RUNNING", null, true])(
    "rejects invalid backend monitoring %s",
    (monitoring) => {
      expect(() => parseClipboardHistoryResult({ items: [], totalCount: 0, monitoring }))
        .toThrow("monitoring");
    }
  );

  it.each(["abc", "01", "-1", "9223372036854775808"])(
    "rejects non-canonical or out-of-range id %s",
    (id) => {
      expect(() => parseClipboardHistoryResult({
        items: [{
          id,
          kind: "text",
          textContent: "内容",
          sourceApplication: null,
          sourceProcess: null,
          capturedAtMs: 1,
          byteSize: 6,
          isFavorite: false
        }],
        totalCount: 1,
        monitoring: "running"
      })).toThrow("id");
    }
  );

  it.each([
    ["text", null],
    ["image", "unexpected text"]
  ] as const)("rejects contradictory %s content", (kind, textContent) => {
    expect(() => parseClipboardHistoryResult({
      items: [{
        id: "1",
        kind,
        textContent,
        sourceApplication: null,
        sourceProcess: null,
        capturedAtMs: 1,
        byteSize: 6,
        isFavorite: false
      }],
      totalCount: 1,
      monitoring: "running"
    })).toThrow("textContent");
  });
});
