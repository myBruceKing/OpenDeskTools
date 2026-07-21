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
      revision: 1,
      kind: "text",
      textContent: "\n  第一行标题  \n第二行内容",
      sourceApplication: null,
      sourceProcess: null,
      capturedAtMs: 1_720_000_000_000,
      byteSize: 32,
      isFavorite: false,
      sourceIconAvailable: false
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
      revision: 1,
      kind: "image",
      textContent: null,
      sourceApplication: null,
      sourceProcess: null,
      capturedAtMs: Number.MAX_SAFE_INTEGER,
      byteSize: 2048,
      isFavorite: true,
      sourceIconAvailable: false
    });
    expect(item.preview).toBe("图片预览暂不可用");
    expect(item.capturedAt).toBe("时间不可用");
  });

  it("accepts file drops only with safe basenames and preserves their display category", () => {
    const parsed = parseClipboardHistoryResult({
      items: [{
        id: "3",
        revision: 1,
        kind: "files",
        textContent: null,
        sourceApplication: "Explorer",
        sourceProcess: "explorer.exe",
        capturedAtMs: 1,
        byteSize: 5,
        isFavorite: false,
        sourceIconAvailable: false,
        fileCount: 1,
        fileNames: ["notes.txt"],
        displayCategory: "text"
      }],
      totalCount: 1,
      monitoring: "running",
      surfaceActive: true,
      inputAvailable: true
    });
    expect(parsed.items[0]).toMatchObject({
      kind: "files",
      fileNames: ["notes.txt"],
      displayCategory: "text"
    });
    expect(toClipboardItemViewModel(parsed.items[0])).toMatchObject({
      kind: "files",
      title: "notes.txt",
      displayCategory: "text"
    });

    for (const fileNames of [["C:\\private\\notes.txt"], ["folder/notes.txt"], []]) {
      expect(() => parseClipboardHistoryResult({
        ...parsed,
        items: [{ ...parsed.items[0], fileNames }]
      })).toThrow();
    }
  });

  it("rejects duplicate ids, invalid counts, and text records without text", () => {
    const valid = {
      id: "1",
      revision: 1,
      kind: "text",
      textContent: "内容",
      sourceApplication: null,
      sourceProcess: null,
      capturedAtMs: 1,
      byteSize: 6,
      isFavorite: false,
      sourceIconAvailable: false
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

  it.each([
    ["clipboard_input_cleanup_failed", "按键释放未确认，输入已暂停。"],
    ["clipboard_files_unavailable", "一个或多个原文件已不存在，无法复制或输入。"]
  ] as const)("maps high-risk %s failures to short nonretryable alerts", (code, message) => {
    expect(normalizeClipboardCommandError({
      code,
      message: "internal Windows failure at C:\\private",
      retryable: true
    })).toEqual({ code, message, retryable: false });
  });

  it("maps an explicit clipboard data write failure as retryable", () => {
    expect(normalizeClipboardCommandError({
      code: "clipboard_write_failed",
      message: "internal",
      retryable: false
    })).toEqual({
      code: "clipboard_write_failed",
      message: "Windows 未完成剪贴板写入，请重试该记录。",
      retryable: true
    });
  });

  it.each(["running", "unavailable"] as const)(
    "accepts the backend monitoring truth %s",
    (monitoring) => {
      expect(parseClipboardHistoryResult({
        items: [], totalCount: 0, monitoring, surfaceActive: false, inputAvailable: false
      }).monitoring)
        .toBe(monitoring);
    }
  );

  it("requires independent surface and target availability truth", () => {
    expect(parseClipboardHistoryResult({
      items: [],
      totalCount: 0,
      monitoring: "running",
      surfaceActive: true,
      inputAvailable: false
    })).toMatchObject({ surfaceActive: true, inputAvailable: false });
    expect(() => parseClipboardHistoryResult({
      items: [], totalCount: 0, monitoring: "running", inputAvailable: false
    })).toThrow("surfaceActive");
    expect(() => parseClipboardHistoryResult({
      items: [], totalCount: 0, monitoring: "running", surfaceActive: false
    })).toThrow("inputAvailable");
  });

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
