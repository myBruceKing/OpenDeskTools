export const MENU_ITEMS_PER_GROUP = 6;
export const WHEEL_CENTER_RADIUS = 36;

const WHEEL_BASE_DIAMETER = 264;
const WHEEL_INNER_ITEM_SIZE = 42;
const WHEEL_OUTER_ITEM_SIZE = 50;
const WHEEL_INNER_RING_RADIUS = 68;
const WHEEL_SINGLE_RING_RADIUS = 84;
const WHEEL_OUTER_RING_RADIUS = 136;
const WHEEL_OUTER_RING_RADIUS_STEP = 72;
const WHEEL_MIN_ARC_SLOT = 84;
const WHEEL_OUTER_PADDING = 10;

export type WheelRing = {
  start: number;
  radius: number;
  itemSize: number;
  capacity: number;
};

export function toolMenuWheelLayout(itemCount: number) {
  const rings: WheelRing[] = [];
  let consumed = 0;
  const isSingleRing = itemCount <= MENU_ITEMS_PER_GROUP;
  for (let ring = 0; consumed < Math.max(1, itemCount); ring += 1) {
    const itemSize = ring === 0 ? WHEEL_INNER_ITEM_SIZE : WHEEL_OUTER_ITEM_SIZE;
    const radius = ring === 0
      ? (isSingleRing ? WHEEL_SINGLE_RING_RADIUS : WHEEL_INNER_RING_RADIUS)
      : WHEEL_OUTER_RING_RADIUS + (ring - 1) * WHEEL_OUTER_RING_RADIUS_STEP;
    const capacity = Math.max(6, Math.floor(2 * Math.PI * radius / WHEEL_MIN_ARC_SLOT));
    rings.push({ start: consumed, radius, itemSize, capacity });
    consumed += capacity;
  }
  const outerRing = rings[rings.length - 1];
  return {
    diameter: Math.max(
      WHEEL_BASE_DIAMETER,
      Math.ceil((outerRing.radius + outerRing.itemSize / 2 + WHEEL_OUTER_PADDING) * 2)
    ),
    rings
  };
}

export function toolMenuWheelPosition(slot: number, ring: WheelRing, diameter: number) {
  const step = 360 / ring.capacity;
  // The first item is at twelve o'clock. Sector dividers sit half a slot
  // away so they never run through an icon.
  const angle = (-90 + slot * step) * Math.PI / 180;
  const radiusPercent = ring.radius / diameter * 100;
  return {
    x: 50 + Math.cos(angle) * radiusPercent,
    y: 50 + Math.sin(angle) * radiusPercent
  };
}
