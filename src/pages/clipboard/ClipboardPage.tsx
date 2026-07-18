import {
  Delete20Regular,
  DocumentTable24Regular,
  DocumentText24Regular,
  Globe24Regular,
  Image24Regular,
  LockClosed16Regular,
  Notebook24Regular,
  Search20Regular,
  Star20Filled,
  Star20Regular
} from "@fluentui/react-icons";
import { useEffect, useMemo, useRef, useState } from "react";
import type { KeyboardEvent } from "react";
import type {
  ClipboardFilter,
  ClipboardItemViewModel,
  ClipboardPageViewModel
} from "../../app/clipboardModel";
import { SplitPane, ThreeColumn } from "../../components/layout/TwoColumn";
import { TagBadge } from "../../components/primitives/Badge";
import { Button } from "../../components/primitives/Button";
import { ConfirmDialog } from "../../components/primitives/Dialog";
import { SearchField, SelectField, TextAreaField, TextField } from "../../components/primitives/Field";
import { HintTooltip } from "../../components/primitives/HintTooltip";
import { SegmentedControl, Toggle } from "../../components/primitives/SelectionControl";
import { List, ListRow } from "../../components/patterns/ListRow";
import { Section, SectionTitle } from "../../components/patterns/Section";
import styles from "./ClipboardPage.module.css";

type ClipboardPageProps = {
  viewModel: ClipboardPageViewModel;
};

const filterOptions: { value: ClipboardFilter; label: string }[] = [
  { value: "all", label: "全部" },
  { value: "text", label: "文本" },
  { value: "image", label: "图片" },
  { value: "favorite", label: "收藏" }
];

const iconByTone = {
  note: Notebook24Regular,
  chrome: Globe24Regular,
  image: Image24Regular,
  excel: DocumentTable24Regular,
  word: DocumentText24Regular,
  snipaste: Image24Regular
} as const;

function kindLabel(kind: ClipboardItemViewModel["kind"]) {
  return kind === "image" ? "图片" : "文本";
}

function historyOptionId(id: string) {
  return `clipboard-history-${id.replace(/[^a-zA-Z0-9_-]/g, "_")}`;
}

function HistoryIcon({ item }: { item: ClipboardItemViewModel }) {
  const Icon = iconByTone[item.iconTone];
  return (
    <span className={[styles.itemIcon, styles[`itemIcon${item.iconTone}`]].join(" ")} aria-hidden="true">
      <Icon />
    </span>
  );
}

function HistoryRow({
  item,
  selected,
  onSelect,
  onToggleFavorite
}: {
  item: ClipboardItemViewModel;
  selected: boolean;
  onSelect: () => void;
  onToggleFavorite: () => void;
}) {
  return (
    <ListRow
      id={historyOptionId(item.id)}
      className={[styles.historyRow, selected ? styles.historyRowSelected : ""].filter(Boolean).join(" ")}
      role="option"
      aria-selected={selected}
      onClick={onSelect}
    >
      <HistoryIcon item={item} />
      <div className={styles.rowCopy}>
        <div className={styles.rowTitle}>{item.title}</div>
        <div className={styles.rowSource}>
          <span className={styles.sourceGlyph} aria-hidden="true" />
          {item.sourceApp}
        </div>
      </div>
      <div className={styles.rowMeta}>
        <span className={styles.rowTime}>{item.time}</span>
        <TagBadge tone={item.kind === "image" ? "green" : "blue"}>{kindLabel(item.kind)}</TagBadge>
      </div>
      <button
        className={styles.favoriteButton}
        type="button"
        aria-label={item.favorite ? "取消收藏" : "收藏"}
        onClick={(event) => {
          event.stopPropagation();
          onToggleFavorite();
        }}
      >
        {item.favorite ? <Star20Filled aria-hidden="true" /> : <Star20Regular aria-hidden="true" />}
      </button>
      {item.locked && <LockClosed16Regular className={styles.lockIcon} aria-hidden="true" />}
    </ListRow>
  );
}

function Toolbar({
  query,
  filter,
  canClearHistory,
  onQueryChange,
  onFilterChange
}: {
  query: string;
  filter: ClipboardFilter;
  canClearHistory: boolean;
  onQueryChange: (value: string) => void;
  onFilterChange: (value: ClipboardFilter) => void;
}) {
  return (
    <div className={styles.toolbar}>
      <SearchField
        className={styles.search}
        data-app-search="true"
        icon={<Search20Regular aria-hidden="true" />}
        shortcut="Ctrl+F"
        placeholder="搜索剪贴板内容"
        value={query}
        onChange={(event) => onQueryChange(event.target.value)}
      />
      <SegmentedControl label="剪贴板筛选" value={filter} options={filterOptions} onChange={onFilterChange} />
      <div className={styles.monitoring}>
        <span>监控已暂停</span>
        <Toggle checked={false} label="剪贴板监控" disabled />
      </div>
      <span className={styles.toolbarDivider} aria-hidden="true" />
      <Button
        className={styles.clearButton}
        variant="text"
        icon={<Delete20Regular aria-hidden="true" />}
        disabled={!canClearHistory}
      >
        清空历史
      </Button>
    </div>
  );
}

function HistoryPanel({
  items,
  totalCount,
  selectedId,
  onSelect,
  onToggleFavorite
}: {
  items: ClipboardItemViewModel[];
  totalCount: number;
  selectedId: string | null;
  onSelect: (id: string) => void;
  onToggleFavorite: (id: string) => void;
}) {
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!selectedId) {
      return;
    }

    document.getElementById(historyOptionId(selectedId))?.scrollIntoView({ block: "nearest" });
  }, [selectedId]);

  const selectAt = (index: number) => {
    const item = items[index];
    if (!item) {
      return;
    }

    onSelect(item.id);
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (items.length === 0) {
      return;
    }

    const selectedIndex = Math.max(0, items.findIndex((item) => item.id === selectedId));

    if (event.key === "ArrowDown") {
      event.preventDefault();
      selectAt(Math.min(items.length - 1, selectedIndex + 1));
    }

    if (event.key === "ArrowUp") {
      event.preventDefault();
      selectAt(Math.max(0, selectedIndex - 1));
    }

    if (event.key === "Home") {
      event.preventDefault();
      selectAt(0);
    }

    if (event.key === "End") {
      event.preventDefault();
      selectAt(items.length - 1);
    }
  };

  return (
    <Section className={styles.historyPanel}>
      <div className={styles.panelHeader}>
        <SectionTitle>剪贴板历史（{totalCount}）</SectionTitle>
        <HintTooltip
          className={styles.panelInfoHint}
          content="滚动查看更多历史项，点击选择；聚焦列表后可用 ↑↓ 切换预览"
          label="查看剪贴板历史提示"
          symbol="i"
        />
      </div>
      <List
        ref={listRef}
        className={styles.historyList}
        role="listbox"
        aria-activedescendant={selectedId ? historyOptionId(selectedId) : undefined}
        aria-label="剪贴板历史"
        tabIndex={0}
        onKeyDown={handleKeyDown}
      >
        {items.map((item) => (
          <HistoryRow
            item={item}
            selected={item.id === selectedId}
            key={item.id}
            onSelect={() => {
              onSelect(item.id);
              listRef.current?.focus();
            }}
            onToggleFavorite={() => onToggleFavorite(item.id)}
          />
        ))}
      </List>
    </Section>
  );
}

function DetailsPanel({
  item,
  viewModel,
  onToggleFavorite,
  onDelete
}: {
  item: ClipboardItemViewModel | null;
  viewModel: ClipboardPageViewModel;
  onToggleFavorite: (id: string) => void;
  onDelete: (id: string) => void;
}) {
  const [deleteConfirmOpen, setDeleteConfirmOpen] = useState(false);
  const infoCopy = item
    ? `来源应用：${item.sourceApp}
来源进程：${item.sourceProcess}
捕获时间：${item.capturedAt}
内容类型：${kindLabel(item.kind)}
大小：${item.size}`
    : "暂无剪贴板内容信息";

  return (
    <Section className={styles.detailsPanel}>
      <div className={styles.panelHeader}>
        <SectionTitle>内容预览</SectionTitle>
        <HintTooltip className={styles.panelInfoHint} content={infoCopy} label="查看内容信息" symbol="i" />
      </div>
      <div className={[styles.previewBox, item?.kind === "image" ? styles.previewBoxImage : ""].filter(Boolean).join(" ")}>
        {item?.kind === "image" && item.previewImageUrl ? (
          <img className={styles.previewImage} src={item.previewImageUrl} alt={item.title} />
        ) : (
          <div className={styles.previewText}>{item?.preview ?? "暂无剪贴板内容"}</div>
        )}
      </div>
      <div className={styles.detailsActions}>
        <Button
          icon={item?.favorite ? <Star20Filled aria-hidden="true" /> : <Star20Regular aria-hidden="true" />}
          disabled={!viewModel.actions.canFavorite || !item}
          onClick={() => {
            if (item) {
              onToggleFavorite(item.id);
            }
          }}
        >
          {item?.favorite ? "取消收藏" : "收藏"}
        </Button>
        <Button
          className={styles.dangerButton}
          icon={<Delete20Regular aria-hidden="true" />}
          disabled={!viewModel.actions.canDelete || !item}
          onClick={() => setDeleteConfirmOpen(true)}
        >
          删除
        </Button>
      </div>
      <ConfirmDialog
        open={deleteConfirmOpen}
        title="删除剪贴板记录"
        description={item ? `确认删除「${item.title}」？当前只从页面预览中移除。` : "没有可删除的剪贴板记录。"}
        confirmText="删除"
        danger
        onConfirm={() => {
          if (item) {
            onDelete(item.id);
          }
          setDeleteConfirmOpen(false);
        }}
        onClose={() => setDeleteConfirmOpen(false)}
      />
    </Section>
  );
}

function SettingsPanel({ viewModel }: { viewModel: ClipboardPageViewModel }) {
  return (
    <Section className={styles.settingsPanel}>
      <ThreeColumn className={styles.settingsGrid} columns="minmax(0, 0.9fr) minmax(0, 1.15fr) minmax(0, 1.7fr)">
        <div className={styles.settingsColumn}>
          <SectionTitle>剪贴板设置</SectionTitle>
          <label className={styles.formRow}>
            <span>保留天数</span>
            <SelectField value={viewModel.settings.retentionDays} disabled>
              <option>{viewModel.settings.retentionDays}</option>
            </SelectField>
          </label>
          <label className={styles.formRow}>
            <span>最大历史数量</span>
            <TextField value={viewModel.settings.maxItems} unit="项" disabled />
          </label>
        </div>
        <div className={styles.settingsColumn}>
          <label className={styles.formRowWide}>
            <span>忽略以下应用（进程名，逗号分隔）</span>
            <span className={styles.inlineField}>
              <TextField value={viewModel.settings.ignoredApps} disabled />
              <Button size="compact" disabled>
                添加
              </Button>
            </span>
          </label>
          <label className={styles.formRowWide}>
            <span>重复内容处理</span>
            <SelectField value={viewModel.settings.duplicateStrategy} disabled>
              <option>{viewModel.settings.duplicateStrategy}</option>
            </SelectField>
          </label>
        </div>
        <div className={[styles.settingsColumn, styles.settingsRulesColumn].join(" ")}>
          <label className={[styles.formRowWide, styles.sensitiveRulesRow].join(" ")}>
            <span className={styles.labelWithHint}>
              敏感内容排除规则（每行一个关键词或正则）
              <HintTooltip content="示例：密码、密钥、token、正则表达式等" />
            </span>
            <TextAreaField className={styles.sensitiveRulesArea} value={viewModel.settings.sensitiveRules} disabled />
          </label>
        </div>
      </ThreeColumn>
    </Section>
  );
}

export function ClipboardPage({ viewModel }: ClipboardPageProps) {
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<ClipboardFilter>("all");
  const [favoriteIds, setFavoriteIds] = useState(() => new Set(viewModel.items.filter((item) => item.favorite).map((item) => item.id)));
  const [deletedIds, setDeletedIds] = useState(() => new Set<string>());
  const [selectedId, setSelectedId] = useState<string | null>(viewModel.items[0]?.id ?? null);

  const items = useMemo(
    () =>
      viewModel.items
        .filter((item) => !deletedIds.has(item.id))
        .map((item) => ({ ...item, favorite: favoriteIds.has(item.id) }))
        .filter((item) => {
          if (filter === "text" && item.kind !== "text") {
            return false;
          }
          if (filter === "image" && item.kind !== "image") {
            return false;
          }
          if (filter === "favorite" && !item.favorite) {
            return false;
          }
          return `${item.title} ${item.preview} ${item.sourceApp}`.toLocaleLowerCase().includes(query.toLocaleLowerCase());
        }),
    [deletedIds, favoriteIds, filter, query, viewModel.items]
  );

  const selectedItem = items.find((item) => item.id === selectedId) ?? items[0] ?? null;
  const totalCount = Math.max(0, viewModel.totalCount - deletedIds.size);
  const toggleFavorite = (id: string) => {
    setFavoriteIds((current) => {
      const next = new Set(current);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  };
  const deleteItem = (id: string) => {
    setDeletedIds((current) => {
      const next = new Set(current);
      next.add(id);
      return next;
    });
  };

  return (
    <div className={styles.page}>
      <Toolbar
        query={query}
        filter={filter}
        canClearHistory={viewModel.actions.canClearHistory}
        onQueryChange={setQuery}
        onFilterChange={setFilter}
      />
      <SplitPane className={styles.middle}>
        <HistoryPanel
          items={items}
          totalCount={totalCount}
          selectedId={selectedItem?.id ?? null}
          onSelect={setSelectedId}
          onToggleFavorite={toggleFavorite}
        />
        <DetailsPanel item={selectedItem} viewModel={viewModel} onToggleFavorite={toggleFavorite} onDelete={deleteItem} />
      </SplitPane>
      <SettingsPanel viewModel={viewModel} />
    </div>
  );
}
