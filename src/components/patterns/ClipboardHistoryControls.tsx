import { Delete20Regular, Star20Filled, Star20Regular } from "@fluentui/react-icons";
import type { KeyboardEvent } from "react";
import type { ClipboardFilter } from "../../app/clipboardModel";
import { SegmentedControl } from "../primitives/SelectionControl";
import styles from "./ClipboardHistoryControls.module.css";

export const clipboardHistoryFilterOptions: { value: ClipboardFilter; label: string }[] = [
  { value: "all", label: "全部" },
  { value: "text", label: "文本" },
  { value: "image", label: "图片" },
  { value: "favorite", label: "收藏" }
];

export function clipboardHistoryFilterLabel(filter: ClipboardFilter) {
  return clipboardHistoryFilterOptions.find((option) => option.value === filter)?.label ?? "全部";
}

export function ClipboardHistoryFilter({
  value,
  onChange
}: {
  value: ClipboardFilter;
  onChange: (value: ClipboardFilter) => void;
}) {
  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const target = (event.target as HTMLElement).closest<HTMLButtonElement>('[role="radio"]');
    if (!target) return;
    const current = clipboardHistoryFilterOptions.findIndex((option) => option.value === value);
    let next = current;
    if (event.key === "ArrowRight" || event.key === "ArrowDown") {
      next = (current + 1) % clipboardHistoryFilterOptions.length;
    } else if (event.key === "ArrowLeft" || event.key === "ArrowUp") {
      next = (current - 1 + clipboardHistoryFilterOptions.length) % clipboardHistoryFilterOptions.length;
    } else if (event.key === "Home") {
      next = 0;
    } else if (event.key === "End") {
      next = clipboardHistoryFilterOptions.length - 1;
    } else {
      return;
    }
    event.preventDefault();
    onChange(clipboardHistoryFilterOptions[next].value);
    event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="radio"]')[next]?.focus();
  };

  return (
    <div
      className={styles.filter}
      data-clipboard-history-filter="true"
      onKeyDown={handleKeyDown}
    >
      <SegmentedControl
        label="剪贴板筛选"
        value={value}
        options={clipboardHistoryFilterOptions}
        onChange={onChange}
      />
    </div>
  );
}

export function ClipboardHistoryRowActions({
  title,
  favorite,
  favoriteDisabled,
  deleteDisabled,
  onToggleFavorite,
  onDelete
}: {
  title: string;
  favorite: boolean;
  favoriteDisabled: boolean;
  deleteDisabled: boolean;
  onToggleFavorite: () => void;
  onDelete: () => void;
}) {
  return (
    <div className={styles.rowActions} data-clipboard-history-actions="true">
      <button
        type="button"
        aria-label={`${favorite ? "取消收藏" : "收藏"} ${title}`}
        disabled={favoriteDisabled}
        onClick={(event) => {
          event.stopPropagation();
          onToggleFavorite();
        }}
        onDoubleClick={(event) => event.stopPropagation()}
      >
        {favorite ? <Star20Filled aria-hidden="true" /> : <Star20Regular aria-hidden="true" />}
      </button>
      <button
        className={styles.deleteButton}
        type="button"
        aria-label={`删除 ${title}`}
        disabled={deleteDisabled}
        onClick={(event) => {
          event.stopPropagation();
          onDelete();
        }}
        onDoubleClick={(event) => event.stopPropagation()}
      >
        <Delete20Regular aria-hidden="true" />
      </button>
    </div>
  );
}
