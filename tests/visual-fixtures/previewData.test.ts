import { describe, expect, it } from "vitest";
import { CLIPBOARD_PREVIEW_DATA, OVERVIEW_PREVIEW_DATA } from "./previewData";

describe("visual fixture harness boundary", () => {
  it("retains representative prototype data only under tests", () => {
    expect(OVERVIEW_PREVIEW_DATA.statistics?.todayTriggers).toBeGreaterThan(0);
    expect(CLIPBOARD_PREVIEW_DATA.items.length).toBeGreaterThan(0);
  });
});
