// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ClipboardItemViewModel } from "../../src/app/clipboardModel";
import { SourceAppIcon } from "../../src/components/patterns/SourceAppIcon";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const item: ClipboardItemViewModel = {
  id: "icon-item-901",
  revision: 3,
  kind: "text",
  title: "内容",
  preview: "内容",
  sourceApp: "记事本",
  sourceProcess: "notepad.exe",
  capturedAt: "2026-07-19 15:00:00",
  time: "15:00:00",
  size: "2 字符",
  favorite: false,
  locked: false,
  privacy: "unknown",
  sourceIconAvailable: true,
  iconTone: "note",
  displayCategory: "text"
};

describe("SourceAppIcon", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
    Object.defineProperty(URL, "createObjectURL", {
      configurable: true,
      value: vi.fn(() => `blob:source-${Math.random()}`)
    });
    Object.defineProperty(URL, "revokeObjectURL", {
      configurable: true,
      value: vi.fn()
    });
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    document.body.replaceChildren();
    vi.clearAllMocks();
  });

  it("shares the lazy blob request and revokes every mounted Object URL", async () => {
    const loadIcon = vi.fn(async () => new Blob([new Uint8Array([1, 2, 3])], { type: "image/png" }));
    await act(async () => {
      root.render(<><SourceAppIcon item={item} loadIcon={loadIcon} /><SourceAppIcon item={item} loadIcon={loadIcon} /></>);
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(loadIcon).toHaveBeenCalledOnce();
    expect(document.querySelectorAll("img")).toHaveLength(2);
    expect(URL.createObjectURL).toHaveBeenCalledTimes(2);

    await act(async () => root.render(<div />));
    expect(URL.revokeObjectURL).toHaveBeenCalledTimes(2);
  });

  it("uses the single fallback without requesting an unavailable icon", async () => {
    const loadIcon = vi.fn(async () => new Blob());
    await act(async () => root.render(<SourceAppIcon item={{ ...item, id: "icon-item-902", sourceIconAvailable: false }} loadIcon={loadIcon} />));
    expect(loadIcon).not.toHaveBeenCalled();
    expect(document.querySelector("[data-source-icon='fallback'] svg")).toBeTruthy();
  });
});
