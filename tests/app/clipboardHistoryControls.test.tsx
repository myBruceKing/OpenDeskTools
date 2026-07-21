// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { readFileSync } from "node:fs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ClipboardHistoryFilter } from "../../src/components/patterns/ClipboardHistoryControls";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const controlsCss = readFileSync("src/components/patterns/ClipboardHistoryControls.module.css", "utf8");

describe("ClipboardHistoryFilter shared sizing", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    document.head.querySelector("style[data-filter-contract]")?.remove();
    document.body.replaceChildren();
  });

  it("computes the same fixed business-control tokens at main and popup widths", async () => {
    await act(async () => root.render(
      <>
        <div style={{ width: 1080 }} data-viewport="main">
          <ClipboardHistoryFilter value="all" onChange={vi.fn()} />
        </div>
        <div style={{ width: 380 }} data-viewport="popup">
          <ClipboardHistoryFilter value="all" onChange={vi.fn()} />
        </div>
      </>
    ));

    const declarations = controlsCss.match(/\.filter\s*\{([\s\S]*?)\}/)?.[1];
    expect(declarations).toBeTruthy();
    const contractStyle = document.createElement("style");
    contractStyle.dataset.filterContract = "true";
    contractStyle.textContent = `[data-clipboard-history-filter="true"] { ${declarations} }`;
    document.head.append(contractStyle);

    const main = document.querySelector<HTMLElement>("[data-viewport='main'] [data-clipboard-history-filter='true']")!;
    const popup = document.querySelector<HTMLElement>("[data-viewport='popup'] [data-clipboard-history-filter='true']")!;
    const mainStyle = getComputedStyle(main);
    const popupStyle = getComputedStyle(popup);

    for (const [property, expected] of [
      ["--segmented-height", "36px"],
      ["--segmented-item-padding-x", "10px"],
      ["--segmented-font-size", "12px"]
    ] as const) {
      expect(mainStyle.getPropertyValue(property).trim()).toBe(expected);
      expect(popupStyle.getPropertyValue(property).trim()).toBe(expected);
    }
    expect(main.querySelector("[role='radiogroup']")?.className)
      .toBe(popup.querySelector("[role='radiogroup']")?.className);
    expect(main.innerHTML.replace(/_r_\w+_/g, "_id_"))
      .toBe(popup.innerHTML.replace(/_r_\w+_/g, "_id_"));
  });
});
