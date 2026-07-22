import { AppGeneric24Regular, Dismiss20Regular } from "@fluentui/react-icons";
import type { CSSProperties } from "react";
import styles from "./ToolMenuPreview.module.css";

export type ToolMenuPreviewItem = {
  id: string;
  label: string;
  iconSrc?: string | null;
};

const MENU_ITEMS_PER_GROUP = 6;
const WHEEL_BASE_DIAMETER = 264;
const WHEEL_ITEM_SIZE = 50;
const WHEEL_RING_RADIUS_STEP = 75;
const WHEEL_MIN_ARC_SLOT = 94;
const WHEEL_CENTER_RADIUS = 41;
const WHEEL_OUTER_PADDING = 12;

type WheelRing = { start: number; radius: number; itemSize: number; capacity: number };

function wheelLayout(itemCount: number) {
  const rings: WheelRing[] = [];
  let consumed = 0;
  for (let ring = 0; consumed < Math.max(1, itemCount); ring += 1) {
    const itemSize = WHEEL_ITEM_SIZE;
    const radius = 77 + ring * WHEEL_RING_RADIUS_STEP;
    const capacity = Math.max(6, Math.floor(2 * Math.PI * radius / WHEEL_MIN_ARC_SLOT));
    rings.push({ start: consumed, radius, itemSize, capacity });
    consumed += capacity;
  }
  return {
    diameter: Math.max(
      WHEEL_BASE_DIAMETER,
      Math.ceil((rings[rings.length - 1].radius + WHEEL_ITEM_SIZE / 2 + WHEEL_OUTER_PADDING) * 2)
    ),
    rings
  };
}

function wheelPosition(slot: number, ring: WheelRing, diameter: number) {
  const step = 360 / ring.capacity;
  // The first item is at twelve o'clock. Sector dividers are derived
  // separately at half a step on either side, so a divider never runs
  // through an icon.
  const angle = (-90 + slot * step) * Math.PI / 180;
  const radiusPercent = ring.radius / diameter * 100;
  return { x: 50 + Math.cos(angle) * radiusPercent, y: 50 + Math.sin(angle) * radiusPercent };
}

type AppIconProps = {
  src?: string | null;
  label: string;
  size?: "row" | "preview" | "compact";
  className?: string;
};

export function AppIcon({ src, label, size = "row", className = "" }: AppIconProps) {
  return (
    <span
      className={[styles.appIcon, styles[`appIcon${size}`], !src ? styles.appIconFallback : "", className]
        .filter(Boolean)
        .join(" ")}
      role="img"
      aria-label={label}
    >
      {src ? <img src={src} alt="" draggable={false} /> : <AppGeneric24Regular aria-hidden="true" />}
    </span>
  );
}

type ToolMenuPreviewProps = {
  variant: "wheel" | "dock" | "vertical";
  items: ToolMenuPreviewItem[];
  size?: "overview" | "settings" | "compact";
  fit?: "content" | "container";
  className?: string;
  onItemClick?: (item: ToolMenuPreviewItem) => void;
};

const rootSizeClass: Record<NonNullable<ToolMenuPreviewProps["size"]>, string> = {
  overview: styles.sizeOverview,
  settings: styles.sizeSettings,
  compact: styles.sizeCompact
};

export function ToolMenuPreview({
  variant,
  items,
  size = "settings",
  fit = "content",
  className = "",
  onItemClick
}: ToolMenuPreviewProps) {
  const visibleItems = items;
  const layout = wheelLayout(visibleItems.length);
  const wheelRingCount = layout.rings.length;
  const iconSize = size === "compact" ? "compact" : "preview";
  const rootClasses = [rootSizeClass[size], fit === "container" ? styles.fitContainer : "", className];

  if (variant === "wheel") {
    return (
      <div
        className={[styles.wheel, ...rootClasses].filter(Boolean).join(" ")}
        style={{
          "--wheel-ring-count": String(wheelRingCount),
          "--wheel-layout-diameter": `${layout.diameter}px`,
          "--wheel-center-size": "82px",
          "--wheel-center-icon-size": "42px",
          "--wheel-layout-center-radius": "41px"
        } as CSSProperties}
        aria-hidden={onItemClick ? undefined : true}
      >
        {layout.rings.flatMap((ring, ringIndex) => {
          const innerBoundary = ringIndex === 0
            ? WHEEL_CENTER_RADIUS
            : (layout.rings[ringIndex - 1].radius + ring.radius) / 2;
          const outerBoundary = ringIndex === layout.rings.length - 1
            ? layout.diameter / 2 - 1
            : (ring.radius + layout.rings[ringIndex + 1].radius) / 2;
          return Array.from({ length: ring.capacity }, (_, slot) => {
            const step = 360 / ring.capacity;
            const angle = -90 - step / 2 + slot * step;
            return (
              <span
                className={styles.wheelSpoke}
                key={`spoke-${ringIndex}-${slot}`}
                style={{
                  "--wheel-spoke-angle": `${angle}deg`,
                  "--wheel-spoke-start": `${innerBoundary}px`,
                  "--wheel-spoke-length": `${outerBoundary - innerBoundary}px`
                } as CSSProperties}
              />
            );
          });
        })}
        {layout.rings.slice(1).map((ring, index) => {
          const previous = layout.rings[index];
          const dividerRadius = (previous.radius + ring.radius) / 2;
          return (
          <span
            className={styles.wheelRingDivider}
            key={`ring-${index}`}
            style={{ "--wheel-ring-divider-size": `${dividerRadius * 2 / layout.diameter * 100}%` } as CSSProperties}
          />
          );
        })}
        {visibleItems.map((item, index) => {
          const ring = layout.rings.findIndex((candidate) => index >= candidate.start && index < candidate.start + candidate.capacity);
          const itemRing = layout.rings[ring];
          const position = wheelPosition(index - itemRing.start, itemRing, layout.diameter);
          const itemStyle = {
            "--wheel-item-x": `${position.x}%`,
            "--wheel-item-y": `${position.y}%`,
            "--wheel-item-size": `${itemRing.itemSize}px`,
            "--wheel-icon-size": `${Math.max(20, itemRing.itemSize - 4)}px`
          } as CSSProperties;
          return onItemClick ? (
            <button
              type="button"
              className={styles.wheelItem}
              key={item.id}
              style={itemStyle}
              data-ring={ring}
              aria-label={`启动 ${item.label}`}
              onClick={() => onItemClick(item)}
            >
              <AppIcon src={item.iconSrc} label={item.label} size={iconSize} />
            </button>
          ) : (
            <span className={styles.wheelItem} key={item.id} style={itemStyle} data-ring={ring}>
              <AppIcon src={item.iconSrc} label={item.label} size={iconSize} />
            </span>
          );
        })}
        <span className={styles.wheelCenter} aria-hidden="true">
          <Dismiss20Regular aria-hidden="true" />
        </span>
      </div>
    );
  }

  if (variant === "vertical") {
    return (
      <div className={[styles.vertical, ...rootClasses].filter(Boolean).join(" ")} aria-hidden={onItemClick ? undefined : true}>
        {visibleItems.map((item) => (
          onItemClick ? (
          <button
            type="button"
            className={styles.verticalItem}
            key={item.id}
            aria-label={`启动 ${item.label}`}
            onClick={() => onItemClick(item)}
          >
            <AppIcon
              src={item.iconSrc}
              label={item.label}
              size={iconSize}
            />
          </button>
          ) : (
          <span className={styles.verticalItem} key={item.id}>
            <AppIcon src={item.iconSrc} label={item.label} size={iconSize} />
          </span>
          )
        ))}
        {visibleItems.length === 0 && <span className={styles.emptyText}>未显示固定项</span>}
      </div>
    );
  }

  return (
    <div
      className={[styles.dock, ...rootClasses]
        .filter(Boolean)
        .join(" ")}
      aria-hidden={onItemClick ? undefined : true}
    >
      {visibleItems.map((item, index) => (
        onItemClick ? (
        <button
          type="button"
          className={styles.dockItem}
          key={item.id}
          data-row-start={index % MENU_ITEMS_PER_GROUP === 0 ? "true" : undefined}
          aria-label={`启动 ${item.label}`}
          onClick={() => onItemClick(item)}
        >
          <AppIcon
            src={item.iconSrc}
            label={item.label}
            size={iconSize}
          />
        </button>
        ) : (
        <span className={styles.dockItem} key={item.id} data-row-start={index % MENU_ITEMS_PER_GROUP === 0 ? "true" : undefined}>
          <AppIcon src={item.iconSrc} label={item.label} size={iconSize} />
        </span>
        )
      ))}
      {visibleItems.length === 0 && <span className={styles.emptyText}>未显示固定项</span>}
    </div>
  );
}
