import { describe, expect, it } from "vitest";
import {
  DISCOVERED_QUICK_LAUNCH_APPS,
  PINNED_QUICK_LAUNCH_APPS,
  toToolMenuPreviewItems
} from "../../src/app/quickLaunchModel";

describe("quick launch production defaults", () => {
  it("starts empty until the native discovery service provides data", () => {
    expect(PINNED_QUICK_LAUNCH_APPS).toEqual([]);
    expect(DISCOVERED_QUICK_LAUNCH_APPS).toEqual([]);
    expect(toToolMenuPreviewItems([])).toEqual([]);
  });
});
