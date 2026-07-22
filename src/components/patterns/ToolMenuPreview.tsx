import { AppGeneric24Regular, Dismiss20Regular } from "@fluentui/react-icons";
import { createPortal } from "react-dom";
import { useLayoutEffect, useRef, useState, type CSSProperties, type PointerEvent as ReactPointerEvent } from "react";
import {
  MENU_ITEMS_PER_GROUP,
  toolMenuWheelLayout,
  toolMenuWheelPosition,
  WHEEL_CENTER_RADIUS
} from "./toolMenuGeometry";
import styles from "./ToolMenuPreview.module.css";

export type ToolMenuPreviewItem = {
  id: string;
  label: string;
  iconSrc?: string | null;
};

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
  onItemReorder?: (active: ToolMenuPreviewItem, over: ToolMenuPreviewItem) => void;
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
  onItemClick,
  onItemReorder
}: ToolMenuPreviewProps) {
  const visibleItems = items;
  const layout = toolMenuWheelLayout(visibleItems.length);
  const wheelRingCount = layout.rings.length;
  const iconSize = size === "compact" ? "compact" : "preview";
  const wheelFitFrameRef = useRef<HTMLDivElement>(null);
  const [wheelPreviewScale, setWheelPreviewScale] = useState(1);
  const [dragState, setDragState] = useState<{
    activeId: string;
    overId: string | null;
    moved: boolean;
    pointerX: number;
    pointerY: number;
  } | null>(null);
  const reorderPointerRef = useRef<{
    pointerId: number;
    item: ToolMenuPreviewItem;
    startX: number;
    startY: number;
    moved: boolean;
    overId: string | null;
  } | null>(null);
  const suppressClickRef = useRef<string | null>(null);
  const rootClasses = [rootSizeClass[size], variant !== "wheel" && fit === "container" ? styles.fitContainer : "", className];

  useLayoutEffect(() => {
    if (variant !== "wheel" || fit !== "container") {
      setWheelPreviewScale(1);
      return undefined;
    }
    const frame = wheelFitFrameRef.current;
    if (!frame) return undefined;
    const updateScale = () => {
      const available = Math.min(frame.clientWidth, frame.clientHeight);
      setWheelPreviewScale(Math.min(1, available / layout.diameter));
    };
    updateScale();
    const observer = new ResizeObserver(updateScale);
    observer.observe(frame);
    return () => observer.disconnect();
  }, [fit, layout.diameter, variant]);

  const interactive = Boolean(onItemClick || onItemReorder);
  const itemById = new Map(visibleItems.map((item) => [item.id, item]));
  const itemClasses = (base: string, item: ToolMenuPreviewItem) => [
    base,
    dragState?.moved && dragState.activeId === item.id ? styles.itemDragging : "",
    dragState?.overId === item.id && dragState.activeId !== item.id ? styles.itemDropTarget : ""
  ].filter(Boolean).join(" ");
  const clearReorder = () => {
    reorderPointerRef.current = null;
    setDragState(null);
  };
  const itemPointerDown = (event: ReactPointerEvent<HTMLElement>, item: ToolMenuPreviewItem) => {
    if (!onItemReorder || event.button !== 0) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    reorderPointerRef.current = {
      pointerId: event.pointerId,
      item,
      startX: event.clientX,
      startY: event.clientY,
      moved: false,
      overId: null
    };
    setDragState({ activeId: item.id, overId: null, moved: false, pointerX: event.clientX, pointerY: event.clientY });
  };
  const itemPointerMove = (event: ReactPointerEvent<HTMLElement>) => {
    const active = reorderPointerRef.current;
    if (!active || active.pointerId !== event.pointerId) return;
    if (!active.moved && Math.hypot(event.clientX - active.startX, event.clientY - active.startY) < 4) return;
    active.moved = true;
    const target = document.elementFromPoint(event.clientX, event.clientY)
      ?.closest<HTMLElement>("[data-tool-menu-preview-item-id]");
    const overId = target?.dataset.toolMenuPreviewItemId ?? null;
    active.overId = overId;
    setDragState({
      activeId: active.item.id,
      overId,
      moved: true,
      pointerX: event.clientX,
      pointerY: event.clientY
    });
  };
  const itemPointerUp = (event: ReactPointerEvent<HTMLElement>) => {
    const active = reorderPointerRef.current;
    if (!active || active.pointerId !== event.pointerId) return;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
    const over = active.overId ? itemById.get(active.overId) : null;
    if (active.moved) {
      suppressClickRef.current = active.item.id;
      if (over && over.id !== active.item.id) onItemReorder?.(active.item, over);
    }
    clearReorder();
  };
  const itemPointerCancel = (event: ReactPointerEvent<HTMLElement>) => {
    if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
    clearReorder();
  };
  const itemClick = (item: ToolMenuPreviewItem) => {
    if (suppressClickRef.current === item.id) {
      suppressClickRef.current = null;
      return;
    }
    onItemClick?.(item);
  };
  const itemInteraction = (item: ToolMenuPreviewItem) => ({
    "data-tool-menu-preview-item-id": item.id,
    onPointerDown: (event: ReactPointerEvent<HTMLElement>) => itemPointerDown(event, item),
    onPointerMove: itemPointerMove,
    onPointerUp: itemPointerUp,
    onPointerCancel: itemPointerCancel,
    onClick: onItemClick ? () => itemClick(item) : undefined
  });
  const activeDrag = dragState?.moved ? dragState : null;
  const draggedItem = activeDrag ? itemById.get(activeDrag.activeId) : null;
  const dragFollower = draggedItem && activeDrag && typeof document !== "undefined"
    ? createPortal(
      <span
        className={styles.dragFollower}
        style={{ left: activeDrag.pointerX, top: activeDrag.pointerY }}
        aria-hidden="true"
      >
        <AppIcon src={draggedItem.iconSrc} label={draggedItem.label} size="preview" />
      </span>,
      document.body
    )
    : null;

  if (variant === "wheel") {
    const wheel = (
      <div
        className={[styles.wheel, fit === "container" ? styles.wheelScaled : "", ...rootClasses].filter(Boolean).join(" ")}
        style={{
          "--wheel-ring-count": String(wheelRingCount),
          "--wheel-layout-diameter": `${layout.diameter}px`,
          "--wheel-center-size": "72px",
          "--wheel-center-icon-size": "36px",
          "--wheel-layout-center-radius": "36px",
          "--wheel-preview-scale": String(wheelPreviewScale)
        } as CSSProperties}
        aria-hidden={interactive ? undefined : true}
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
          const position = toolMenuWheelPosition(index - itemRing.start, itemRing, layout.diameter);
          const itemStyle = {
            "--wheel-item-x": `${position.x}%`,
            "--wheel-item-y": `${position.y}%`,
            "--wheel-item-size": `${itemRing.itemSize}px`,
            "--wheel-icon-size": `${Math.max(20, itemRing.itemSize - 4)}px`
          } as CSSProperties;
          return interactive ? (
            <button
              type="button"
              className={itemClasses(styles.wheelItem, item)}
              key={item.id}
              style={itemStyle}
              data-ring={ring}
              aria-label={`启动 ${item.label}`}
              {...itemInteraction(item)}
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
    return fit === "container" ? <>{<div className={styles.wheelFitFrame} ref={wheelFitFrameRef}>{wheel}</div>}{dragFollower}</> : <>{wheel}{dragFollower}</>;
  }

  if (variant === "vertical") {
    return (
      <>
      <div className={[styles.vertical, ...rootClasses].filter(Boolean).join(" ")} aria-hidden={interactive ? undefined : true}>
        {visibleItems.map((item) => (
          interactive ? (
          <button
            type="button"
            className={itemClasses(styles.verticalItem, item)}
            key={item.id}
            aria-label={`启动 ${item.label}`}
            {...itemInteraction(item)}
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
      {dragFollower}
      </>
    );
  }

  return (
    <>
    <div
      className={[styles.dock, ...rootClasses]
        .filter(Boolean)
        .join(" ")}
      aria-hidden={interactive ? undefined : true}
    >
      {visibleItems.map((item, index) => (
        interactive ? (
        <button
          type="button"
          className={itemClasses(styles.dockItem, item)}
          key={item.id}
          data-row-start={index % MENU_ITEMS_PER_GROUP === 0 ? "true" : undefined}
          aria-label={`启动 ${item.label}`}
          {...itemInteraction(item)}
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
    {dragFollower}
    </>
  );
}
