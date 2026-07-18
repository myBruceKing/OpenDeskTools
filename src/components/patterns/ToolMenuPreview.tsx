import { AppGeneric24Regular, Dismiss20Regular } from "@fluentui/react-icons";
import type { CSSProperties } from "react";
import styles from "./ToolMenuPreview.module.css";

export type ToolMenuPreviewItem = {
  id: string;
  label: string;
  iconSrc?: string | null;
};

const wheelPositions = [
  { x: 50, y: 16 },
  { x: 78, y: 34 },
  { x: 78, y: 67 },
  { x: 50, y: 84 },
  { x: 22, y: 67 },
  { x: 22, y: 34 }
];

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
  className = ""
}: ToolMenuPreviewProps) {
  const visibleItems = items.slice(0, 6);
  const iconSize = size === "compact" ? "compact" : "preview";
  const rootClasses = [rootSizeClass[size], fit === "container" ? styles.fitContainer : "", className];

  if (variant === "wheel") {
    return (
      <div className={[styles.wheel, ...rootClasses].filter(Boolean).join(" ")} aria-hidden="true">
        {visibleItems.map((item, index) => {
          const position = wheelPositions[index] ?? wheelPositions[0];
          return (
            <span
              className={styles.wheelItem}
              key={item.id}
              style={{ "--wheel-item-x": `${position.x}%`, "--wheel-item-y": `${position.y}%` } as CSSProperties}
            >
              <AppIcon src={item.iconSrc} label={item.label} size={iconSize} />
            </span>
          );
        })}
        <span className={styles.wheelCenter}>
          <Dismiss20Regular aria-hidden="true" />
        </span>
      </div>
    );
  }

  if (variant === "vertical") {
    return (
      <div className={[styles.vertical, ...rootClasses].filter(Boolean).join(" ")} aria-hidden="true">
        {visibleItems.map((item) => (
          <span className={styles.verticalItem} key={item.id}>
            <AppIcon
              src={item.iconSrc}
              label={item.label}
              size={iconSize}
            />
          </span>
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
      aria-hidden="true"
    >
      {visibleItems.map((item) => (
        <span className={styles.dockItem} key={item.id}>
          <AppIcon
            src={item.iconSrc}
            label={item.label}
            size={iconSize}
          />
        </span>
      ))}
      {visibleItems.length === 0 && <span className={styles.emptyText}>未显示固定项</span>}
    </div>
  );
}
