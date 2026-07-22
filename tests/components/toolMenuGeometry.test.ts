import { describe, expect, it } from "vitest";
import {
  MENU_ITEMS_PER_GROUP,
  toolMenuWheelLayout,
  toolMenuWheelPosition
} from "../../src/components/patterns/toolMenuGeometry";

describe("tool menu geometry", () => {
  it("keeps one group on a single six-slot ring", () => {
    const layout = toolMenuWheelLayout(MENU_ITEMS_PER_GROUP);

    expect(layout.rings).toHaveLength(1);
    expect(layout.rings[0].capacity).toBe(MENU_ITEMS_PER_GROUP);
  });

  it("moves the seventh item to an outer ring without shrinking that ring", () => {
    const layout = toolMenuWheelLayout(MENU_ITEMS_PER_GROUP + 1);

    expect(layout.rings).toHaveLength(2);
    expect(layout.rings[1].itemSize).toBeGreaterThan(layout.rings[0].itemSize);
    expect(layout.diameter).toBeGreaterThan(toolMenuWheelLayout(MENU_ITEMS_PER_GROUP).diameter);
  });

  it("places the first slot at twelve o'clock", () => {
    const layout = toolMenuWheelLayout(1);
    const position = toolMenuWheelPosition(0, layout.rings[0], layout.diameter);

    expect(position.x).toBeCloseTo(50);
    expect(position.y).toBeLessThan(50);
  });
});
