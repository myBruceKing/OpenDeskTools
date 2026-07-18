import { describe, expect, it } from "vitest";
import {
  EMPTY_CLIPBOARD_VIEW_MODEL,
  getClipboardMonitoringPresentation
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
});
