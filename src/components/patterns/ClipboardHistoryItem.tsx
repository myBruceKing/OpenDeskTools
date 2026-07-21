import {
  Copy20Regular,
  Delete20Regular,
  LockClosed16Regular,
  Star20Filled,
  Star20Regular
} from "@fluentui/react-icons";
import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent as ReactKeyboardEvent,
  type KeyboardEventHandler,
  type MouseEventHandler,
  type Ref
} from "react";
import type { ClipboardItemViewModel } from "../../app/clipboardModel";
import { TagBadge } from "../primitives/Badge";
import { ImagePreview, type ImagePreviewState } from "./ImagePreview";
import { ClipboardHistoryRowActions } from "./ClipboardHistoryControls";
import { SourceAppIcon, type LoadClipboardSourceIcon } from "./SourceAppIcon";
import styles from "./ClipboardHistoryItem.module.css";

export function clipboardHistoryKindLabel(kind: ClipboardItemViewModel["kind"]) {
  if (kind === "image") return "图片";
  if (kind === "files") return "文件";
  return "文本";
}

export function clipboardHistoryPreviewLabel(kind: ClipboardItemViewModel["kind"]) {
  if (kind === "image") return "图片预览";
  if (kind === "files") return "文件列表";
  return "文本预览";
}

export function clipboardHistoryInfoCopy(item: ClipboardItemViewModel | null) {
  if (!item) return "暂无剪贴板内容信息";
  return `来源应用：${item.sourceApp}\n来源进程：${item.sourceProcess}\n捕获时间：${item.capturedAt}\n内容类型：${clipboardHistoryKindLabel(item.kind)}\n大小：${item.size}`;
}

export function ClipboardHistoryRowContent({
  item,
  loadSourceIcon,
  favoriteDisabled,
  deleteDisabled,
  onToggleFavorite,
  onDelete,
  className
}: {
  item: ClipboardItemViewModel;
  loadSourceIcon: LoadClipboardSourceIcon;
  favoriteDisabled: boolean;
  deleteDisabled: boolean;
  onToggleFavorite: () => void;
  onDelete: () => void;
  className?: string;
}) {
  return (
    <div
      className={[styles.rowContent, className].filter(Boolean).join(" ")}
      data-clipboard-history-row-content="true"
    >
      <SourceAppIcon className={styles.sourceIcon} item={item} loadIcon={loadSourceIcon} />
      <div className={styles.rowCopy}>
        <div className={styles.rowTitle}>{item.title}</div>
        <div className={styles.rowSource}>
          <span>{item.sourceApp}</span>
          {item.locked && <LockClosed16Regular className={styles.lockIcon} aria-hidden="true" />}
        </div>
      </div>
      <div className={styles.rowMeta}>
        <time>{item.time}</time>
        <TagBadge tone={item.displayCategory === "image" ? "green" : item.kind === "files" ? "warning" : "blue"}>
          {clipboardHistoryKindLabel(item.kind)}
        </TagBadge>
      </div>
      <ClipboardHistoryRowActions
        title={item.title}
        favorite={item.favorite}
        favoriteDisabled={favoriteDisabled}
        deleteDisabled={deleteDisabled}
        onToggleFavorite={onToggleFavorite}
        onDelete={onDelete}
      />
    </div>
  );
}

export function ClipboardHistoryPreviewContent({
  item,
  imagePreview,
  emptyLabel = "暂无剪贴板内容",
  className,
  textRef,
  textRole,
  textTabIndex,
  textAriaLabel,
  onTextDoubleClick,
  onTextKeyDown,
  onImageLoaded,
  onImageError,
  onRetryImage
}: {
  item: ClipboardItemViewModel | null;
  imagePreview?: ImagePreviewState;
  emptyLabel?: string;
  className?: string;
  textRef?: Ref<HTMLDivElement>;
  textRole?: "button";
  textTabIndex?: number;
  textAriaLabel?: string;
  onTextDoubleClick?: MouseEventHandler<HTMLDivElement>;
  onTextKeyDown?: KeyboardEventHandler<HTMLDivElement>;
  onImageLoaded?: (url: string) => void;
  onImageError?: (url: string) => void;
  onRetryImage?: () => void;
}) {
  return (
    <div
      className={[styles.previewContent, className].filter(Boolean).join(" ")}
      data-clipboard-history-preview-content="true"
    >
      {item?.kind === "image" && imagePreview ? (
        <ImagePreview
          state={imagePreview}
          alt={`剪贴板图片，来源于${item.sourceApp}，捕获于${item.capturedAt}`}
          onLoad={onImageLoaded ?? (() => undefined)}
          onError={onImageError ?? (() => undefined)}
          onRetry={onRetryImage ?? (() => undefined)}
        />
      ) : (
        <div
          ref={textRef}
          className={styles.previewText}
          role={textRole}
          tabIndex={textTabIndex}
          aria-label={textAriaLabel}
          onDoubleClick={onTextDoubleClick}
          onKeyDown={onTextKeyDown}
        >
          {item?.preview ?? emptyLabel}
        </div>
      )}
    </div>
  );
}

export type ClipboardHistoryMenuCloseReason = "outside" | "blur" | "viewport" | "keyboard";

export function ClipboardHistoryContextMenu({
  item,
  point,
  initialFocus,
  onCopy,
  onToggleFavorite,
  onDelete,
  onClose
}: {
  item: ClipboardItemViewModel;
  point: { x: number; y: number };
  initialFocus: boolean;
  onCopy: () => void;
  onToggleFavorite: () => void;
  onDelete: () => void;
  onClose: (reason: ClipboardHistoryMenuCloseReason) => void;
}) {
  const menuRef = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState(point);

  useLayoutEffect(() => {
    const menu = menuRef.current;
    if (!menu) return;
    const rect = menu.getBoundingClientRect();
    setPosition({
      x: Math.max(8, Math.min(point.x, window.innerWidth - rect.width - 8)),
      y: Math.max(8, Math.min(point.y, window.innerHeight - rect.height - 8))
    });
    if (initialFocus) {
      menu.querySelector<HTMLButtonElement>('button[role="menuitem"]')?.focus();
    }
  }, [initialFocus, point]);

  useEffect(() => {
    const closeOutside = (event: Event) => {
      if (!menuRef.current?.contains(event.target as Node)) onClose("outside");
    };
    const closeOnBlur = () => onClose("blur");
    const closeOnViewportChange = () => onClose("viewport");
    const closeWhenHidden = () => {
      if (document.visibilityState === "hidden") onClose("blur");
    };
    document.addEventListener("pointerdown", closeOutside, true);
    document.addEventListener("click", closeOutside, true);
    document.addEventListener("visibilitychange", closeWhenHidden);
    window.addEventListener("blur", closeOnBlur);
    window.addEventListener("resize", closeOnViewportChange);
    return () => {
      document.removeEventListener("pointerdown", closeOutside, true);
      document.removeEventListener("click", closeOutside, true);
      document.removeEventListener("visibilitychange", closeWhenHidden);
      window.removeEventListener("blur", closeOnBlur);
      window.removeEventListener("resize", closeOnViewportChange);
    };
  }, [onClose]);

  const handleKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    const buttons = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="menuitem"]'));
    const current = buttons.indexOf(document.activeElement as HTMLButtonElement);
    let next = current;
    if (event.key === "ArrowDown") next = (current + 1 + buttons.length) % buttons.length;
    else if (event.key === "ArrowUp") next = (current - 1 + buttons.length) % buttons.length;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = buttons.length - 1;
    else if (event.key === "Escape" || event.key === "Tab") {
      event.preventDefault();
      onClose("keyboard");
      return;
    } else return;
    event.preventDefault();
    buttons[next]?.focus();
  };

  return (
    <div
      ref={menuRef}
      className={styles.contextMenu}
      style={{ left: position.x, top: position.y } as CSSProperties}
      role="menu"
      aria-label={`${item.title}操作`}
      onKeyDown={handleKeyDown}
      onBlur={(event) => {
        if (event.relatedTarget && !event.currentTarget.contains(event.relatedTarget as Node)) onClose("blur");
      }}
    >
      <button type="button" role="menuitem" onClick={onCopy}><Copy20Regular aria-hidden="true" />复制</button>
      <button type="button" role="menuitem" onClick={onToggleFavorite}>
        {item.favorite ? <Star20Filled aria-hidden="true" /> : <Star20Regular aria-hidden="true" />}
        {item.favorite ? "取消收藏" : "收藏"}
      </button>
      <button type="button" className={styles.contextDanger} role="menuitem" onClick={onDelete}>
        <Delete20Regular aria-hidden="true" />删除
      </button>
    </div>
  );
}
