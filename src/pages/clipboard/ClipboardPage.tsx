import {
  CheckmarkCircle20Regular,
  Delete20Regular,
  Search20Regular,
  Warning20Regular
} from "@fluentui/react-icons";
import { useEffect, useMemo, useRef, useState } from "react";
import type { KeyboardEvent } from "react";
import type {
  ClipboardControllerState,
  ClipboardFilter,
  ClipboardItemViewModel,
  ClipboardMonitoringState
} from "../../app/clipboardModel";
import { getClipboardMonitoringPresentation } from "../../app/clipboardModel";
import { SplitPane, ThreeColumn } from "../../components/layout/TwoColumn";
import { Button } from "../../components/primitives/Button";
import { ConfirmDialog } from "../../components/primitives/Dialog";
import { SearchField, SelectField, TextAreaField, TextField } from "../../components/primitives/Field";
import { HintTooltip } from "../../components/primitives/HintTooltip";
import { Toggle } from "../../components/primitives/SelectionControl";
import { List, ListRow } from "../../components/patterns/ListRow";
import { type ImagePreviewState } from "../../components/patterns/ImagePreview";
import { ClipboardHistoryFilter } from "../../components/patterns/ClipboardHistoryControls";
import {
  ClipboardHistoryPreviewContent,
  ClipboardHistoryRowContent,
  clipboardHistoryInfoCopy
} from "../../components/patterns/ClipboardHistoryItem";
import { Section, SectionTitle } from "../../components/patterns/Section";
import { type LoadClipboardSourceIcon } from "../../components/patterns/SourceAppIcon";
import styles from "./ClipboardPage.module.css";
import { useClipboardImagePreview, type LoadClipboardImage } from "./useClipboardImagePreview";

type ClipboardPageProps = {
  state: ClipboardControllerState;
  loadImage: LoadClipboardImage;
  loadSourceIcon: LoadClipboardSourceIcon;
  onUpdateText: (id: string, textContent: string, expectedRevision: number) => Promise<boolean>;
  onSetFavorite: (id: string, isFavorite: boolean) => void;
  onDelete: (id: string) => void;
  onClearUnfavoriteHistory: () => void;
};

function historyOptionId(id: string) {
  return `clipboard-history-${id.replace(/[^a-zA-Z0-9_-]/g, "_")}`;
}

function HistoryRow({
  item,
  selected,
  favoriteDisabled,
  deleteDisabled,
  loadSourceIcon,
  onSelect,
  onToggleFavorite,
  onDelete
}: {
  item: ClipboardItemViewModel;
  selected: boolean;
  favoriteDisabled: boolean;
  deleteDisabled: boolean;
  loadSourceIcon: LoadClipboardSourceIcon;
  onSelect: () => void;
  onToggleFavorite: () => void;
  onDelete: () => void;
}) {
  return (
    <ListRow
      id={historyOptionId(item.id)}
      className={[styles.historyRow, selected ? styles.historyRowSelected : ""].filter(Boolean).join(" ")}
      role="option"
      aria-selected={selected}
      onClick={onSelect}
    >
      <ClipboardHistoryRowContent
        className={styles.historyRowContent}
        item={item}
        loadSourceIcon={loadSourceIcon}
        favoriteDisabled={favoriteDisabled}
        deleteDisabled={deleteDisabled}
        onToggleFavorite={onToggleFavorite}
        onDelete={onDelete}
      />
    </ListRow>
  );
}

function Toolbar({
  query,
  filter,
  monitoring,
  canClearHistory,
  clearing,
  onQueryChange,
  onFilterChange,
  onClearHistory
}: {
  query: string;
  filter: ClipboardFilter;
  monitoring: ClipboardMonitoringState;
  canClearHistory: boolean;
  clearing: boolean;
  onQueryChange: (value: string) => void;
  onFilterChange: (value: ClipboardFilter) => void;
  onClearHistory: () => void;
}) {
  const monitoringPresentation = getClipboardMonitoringPresentation(monitoring);

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
      <ClipboardHistoryFilter value={filter} onChange={onFilterChange} />
      <div className={styles.monitoring}>
        <span>{monitoringPresentation.label}</span>
        <Toggle
          checked={monitoringPresentation.checked}
          label="剪贴板监控"
          disabled={monitoringPresentation.disabled}
        />
      </div>
      <span className={styles.toolbarDivider} aria-hidden="true" />
      <Button
        className={styles.clearButton}
        variant="text"
        icon={<Delete20Regular aria-hidden="true" />}
        disabled={!canClearHistory || clearing}
        onClick={onClearHistory}
      >
        {clearing ? "正在清空" : "清空历史"}
      </Button>
    </div>
  );
}

function HistoryPanel({
  items,
  totalCount,
  selectedId,
  statusMessage,
  statusIsError,
  pendingItemIds,
  canFavorite,
  canDelete,
  loadSourceIcon,
  onSelect,
  onToggleFavorite,
  onDelete
}: {
  items: ClipboardItemViewModel[];
  totalCount: number;
  selectedId: string | null;
  statusMessage: string | null;
  statusIsError: boolean;
  pendingItemIds: readonly string[];
  canFavorite: boolean;
  canDelete: boolean;
  loadSourceIcon: LoadClipboardSourceIcon;
  onSelect: (id: string) => void;
  onToggleFavorite: (id: string) => void;
  onDelete: (id: string) => void;
}) {
  const listRef = useRef<HTMLDivElement>(null);
  const [deleteId, setDeleteId] = useState<string | null>(null);
  const deleteItem = deleteId ? items.find((item) => item.id === deleteId) ?? null : null;

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
          content="点击选择；聚焦列表后可用 ↑↓、Home、End 切换预览"
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
        {statusMessage && (
          <div className={styles.historyNotice} role={statusIsError ? "alert" : "status"}>
            {statusMessage}
          </div>
        )}
        {items.length === 0 ? (
          !statusMessage && <div className={styles.emptyHistory}>暂无剪贴板历史</div>
        ) : (
          items.map((item) => (
            <HistoryRow
              item={item}
              selected={item.id === selectedId}
              favoriteDisabled={!canFavorite || pendingItemIds.includes(item.id)}
              deleteDisabled={!canDelete || pendingItemIds.includes(item.id)}
              loadSourceIcon={loadSourceIcon}
              key={item.id}
              onSelect={() => {
                onSelect(item.id);
                listRef.current?.focus();
              }}
              onToggleFavorite={() => onToggleFavorite(item.id)}
              onDelete={() => setDeleteId(item.id)}
            />
          ))
        )}
      </List>
      <ConfirmDialog
        open={deleteItem !== null}
        title="删除剪贴板记录"
        description={deleteItem ? `确认永久删除「${deleteItem.title}」？` : ""}
        confirmText="删除"
        danger
        onConfirm={() => {
          if (deleteItem) onDelete(deleteItem.id);
          setDeleteId(null);
        }}
        onClose={() => setDeleteId(null)}
      />
    </Section>
  );
}

function DetailsPanel({
  item,
  imagePreview,
  canEditText,
  itemPending,
  textEdit,
  onImageLoaded,
  onImageError,
  onRetryImage,
  onUpdateText
}: {
  item: ClipboardItemViewModel | null;
  imagePreview: ImagePreviewState;
  canEditText: boolean;
  itemPending: boolean;
  textEdit: ClipboardControllerState["textEdit"];
  onImageLoaded: (url: string) => void;
  onImageError: (url: string) => void;
  onRetryImage: () => void;
  onUpdateText: (id: string, textContent: string, expectedRevision: number) => Promise<boolean>;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(item?.kind === "text" ? item.preview : "");
  const previewRef = useRef<HTMLDivElement>(null);
  const editorRef = useRef<HTMLTextAreaElement>(null);
  const savingRef = useRef(false);
  const activeEdit = item && textEdit?.itemId === item.id ? textEdit : null;
  const editPending = activeEdit?.status === "pending";
  const infoCopy = clipboardHistoryInfoCopy(item);

  useEffect(() => {
    setEditing(false);
    savingRef.current = false;
    setDraft(item?.kind === "text" ? item.preview : "");
  }, [item?.id]);

  useEffect(() => {
    if (editing && !editPending) {
      editorRef.current?.focus();
    }
  }, [editPending, editing]);

  useEffect(() => {
    if (activeEdit?.code !== "clipboard_revision_conflict" || !editing) {
      return;
    }
    setEditing(false);
    setDraft(item?.kind === "text" ? item.preview : "");
    window.requestAnimationFrame(() => previewRef.current?.focus());
  }, [activeEdit?.code, editing, item?.kind, item?.preview]);

  const beginEditing = () => {
    if (!item || item.kind !== "text" || !canEditText || itemPending || editPending) {
      return;
    }
    setDraft(item.preview);
    setEditing(true);
  };

  const cancelEditing = () => {
    if (!item || editPending) {
      return;
    }
    setDraft(item.kind === "text" ? item.preview : "");
    setEditing(false);
    window.requestAnimationFrame(() => previewRef.current?.focus());
  };

  const saveEditing = async () => {
    if (!item || item.kind !== "text" || savingRef.current || editPending) {
      return;
    }
    if (draft === item.preview) {
      setEditing(false);
      window.requestAnimationFrame(() => previewRef.current?.focus());
      return;
    }
    savingRef.current = true;
    const saved = await onUpdateText(item.id, draft, item.revision);
    savingRef.current = false;
    if (saved) {
      setEditing(false);
      window.requestAnimationFrame(() => previewRef.current?.focus());
    }
  };

  return (
    <Section className={styles.detailsPanel}>
      <div className={styles.panelHeader}>
        <SectionTitle>内容预览</SectionTitle>
        <HintTooltip
          className={styles.panelInfoHint}
          content={infoCopy}
          label="查看内容信息"
          symbol="i"
          interactive
        />
      </div>
      <div className={styles.detailsContent}>
        <div className={styles.previewBox}>
          {editing && item ? (
            <textarea
              ref={editorRef}
              className={styles.previewEditor}
              aria-label="编辑剪贴板文本"
              aria-describedby={activeEdit ? "clipboard-edit-feedback" : undefined}
              value={draft}
              disabled={editPending}
              onChange={(event) => setDraft(event.target.value)}
              onBlur={() => void saveEditing()}
              onKeyDown={(event) => {
                if (event.key === "Escape") {
                  event.preventDefault();
                  event.stopPropagation();
                  cancelEditing();
                } else if (event.key === "Enter" && event.ctrlKey) {
                  event.preventDefault();
                  void saveEditing();
                }
              }}
            />
          ) : (
            <ClipboardHistoryPreviewContent
              item={item}
              imagePreview={imagePreview}
              textRef={previewRef}
              textRole={item?.kind === "text" ? "button" : undefined}
              textTabIndex={item?.kind === "text" ? 0 : undefined}
              textAriaLabel={item?.kind === "text" ? "双击编辑剪贴板文本" : undefined}
              onTextDoubleClick={beginEditing}
              onTextKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault();
                  beginEditing();
                }
              }}
              onImageLoaded={onImageLoaded}
              onImageError={onImageError}
              onRetryImage={onRetryImage}
            />
          )}
        </div>
        {activeEdit && <div className={styles.detailsFooter}>
          <div
            className={[
              styles.actionFeedback,
              styles[`actionFeedback${activeEdit.status === "error" ? "error" : activeEdit.status === "success" ? "success" : "neutral"}`]
            ].filter(Boolean).join(" ")}
            id="clipboard-edit-feedback"
            role={activeEdit?.status === "error" ? "alert" : "status"}
            aria-live="polite"
          >
            {activeEdit?.status === "success" && <CheckmarkCircle20Regular aria-hidden="true" />}
            {activeEdit?.status === "error" && <Warning20Regular aria-hidden="true" />}
            <span>{activeEdit.message}</span>
          </div>
        </div>}
      </div>
    </Section>
  );
}

function SettingsPanel({ viewModel }: { viewModel: ClipboardControllerState["viewModel"] }) {
  const unavailableValue = "—";
  const retentionDays = viewModel.settings.retentionDays ?? unavailableValue;
  const duplicateStrategy = viewModel.settings.duplicateStrategy ?? unavailableValue;

  return (
    <Section className={styles.settingsPanel}>
      <ThreeColumn className={styles.settingsGrid} columns="minmax(0, 0.9fr) minmax(0, 1.15fr) minmax(0, 1.7fr)">
        <div className={styles.settingsColumn}>
          <SectionTitle>剪贴板设置</SectionTitle>
          <label className={styles.formRow}>
            <span>保留天数</span>
            <SelectField value={retentionDays} disabled>
              <option>{retentionDays}</option>
            </SelectField>
          </label>
          <label className={styles.formRow}>
            <span>最大历史数量</span>
            <TextField
              value={viewModel.settings.maxItems ?? unavailableValue}
              unit={viewModel.settings.maxItems === null ? undefined : "项"}
              disabled
            />
          </label>
        </div>
        <div className={styles.settingsColumn}>
          <label className={styles.formRowWide}>
            <span>忽略以下应用（进程名，逗号分隔）</span>
            <span className={styles.inlineField}>
              <TextField value={viewModel.settings.ignoredApps ?? unavailableValue} disabled />
              <Button size="compact" disabled>
                添加
              </Button>
            </span>
          </label>
          <label className={styles.formRowWide}>
            <span>重复内容处理</span>
            <SelectField value={duplicateStrategy} disabled>
              <option>{duplicateStrategy}</option>
            </SelectField>
          </label>
        </div>
        <div className={[styles.settingsColumn, styles.settingsRulesColumn].join(" ")}>
          <label className={[styles.formRowWide, styles.sensitiveRulesRow].join(" ")}>
            <span className={styles.labelWithHint}>
              敏感内容排除规则（每行一个关键词或正则）
              <HintTooltip content="示例：密码、密钥、token、正则表达式等" />
            </span>
            <TextAreaField
              className={styles.sensitiveRulesArea}
              value={viewModel.settings.sensitiveRules ?? ""}
              placeholder={viewModel.settings.sensitiveRules === null ? unavailableValue : undefined}
              disabled
            />
          </label>
        </div>
      </ThreeColumn>
    </Section>
  );
}

export function ClipboardPage({
  state,
  loadImage,
  loadSourceIcon,
  onUpdateText,
  onSetFavorite,
  onDelete,
  onClearUnfavoriteHistory
}: ClipboardPageProps) {
  const { viewModel } = state;
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<ClipboardFilter>("all");
  const [clearConfirmOpen, setClearConfirmOpen] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(viewModel.items[0]?.id ?? null);

  const items = useMemo(
    () =>
      viewModel.items
        .filter((item) => {
          if (filter === "text" && item.displayCategory !== "text") {
            return false;
          }
          if (filter === "image" && item.displayCategory !== "image") {
            return false;
          }
          if (filter === "favorite" && !item.favorite) {
            return false;
          }
          return `${item.title} ${item.preview} ${item.sourceApp}`.toLocaleLowerCase().includes(query.toLocaleLowerCase());
        }),
    [filter, query, viewModel.items]
  );

  const selectedItem = items.find((item) => item.id === selectedId) ?? items[0] ?? null;
  const imagePreview = useClipboardImagePreview(
    selectedItem?.kind === "image" ? selectedItem.id : null,
    loadImage
  );
  const toggleFavorite = (id: string) => {
    const item = viewModel.items.find((candidate) => candidate.id === id);
    if (item) {
      onSetFavorite(id, !item.favorite);
    }
  };
  const emptyStatusMessage = state.status === "loading"
    ? "正在加载剪贴板历史…"
    : state.status === "unavailable"
      ? "剪贴板历史不可用"
      : null;
  const statusMessage = state.error?.message ?? state.realtimeError?.message ?? emptyStatusMessage;
  const actionsAvailable = state.status === "ready" && !state.clearing;
  const canFavorite = viewModel.actions.canFavorite && actionsAvailable;
  const canDelete = viewModel.actions.canDelete && actionsAvailable;
  const canClearHistory = viewModel.actions.canClearHistory
    && state.status === "ready"
    && state.pendingItemIds.length === 0
    && viewModel.items.some((item) => !item.favorite);
  const selectedPending = selectedItem
    ? state.pendingItemIds.includes(selectedItem.id)
    : false;
  const canEditText = viewModel.actions.canEditText && actionsAvailable;

  return (
    <div className={styles.page}>
      <Toolbar
        query={query}
        filter={filter}
        monitoring={viewModel.monitoring}
        canClearHistory={canClearHistory}
        clearing={state.clearing}
        onQueryChange={setQuery}
        onFilterChange={setFilter}
        onClearHistory={() => setClearConfirmOpen(true)}
      />
      <SplitPane className={styles.middle}>
        <HistoryPanel
          items={items}
          totalCount={viewModel.totalCount}
          selectedId={selectedItem?.id ?? null}
          statusMessage={statusMessage}
          statusIsError={state.error !== null || state.realtimeError !== null || state.status === "unavailable"}
          pendingItemIds={state.pendingItemIds}
          canFavorite={canFavorite}
          canDelete={canDelete}
          loadSourceIcon={loadSourceIcon}
          onSelect={setSelectedId}
          onToggleFavorite={toggleFavorite}
          onDelete={(id) => {
            if (selectedItem?.id === id && selectedItem.kind === "image") {
              imagePreview.release();
            }
            onDelete(id);
          }}
        />
        <DetailsPanel
          item={selectedItem}
          imagePreview={imagePreview.state}
          canEditText={canEditText}
          itemPending={selectedPending}
          textEdit={state.textEdit}
          onImageLoaded={imagePreview.markLoaded}
          onImageError={imagePreview.markDecodeError}
          onRetryImage={imagePreview.retry}
          onUpdateText={onUpdateText}
        />
      </SplitPane>
      <SettingsPanel viewModel={viewModel} />
      <ConfirmDialog
        open={clearConfirmOpen}
        title="清空未收藏历史"
        description="确认永久删除全部未收藏的剪贴板记录？已收藏内容会保留。"
        confirmText="清空"
        danger
        onConfirm={() => {
          imagePreview.release();
          onClearUnfavoriteHistory();
          setClearConfirmOpen(false);
        }}
        onClose={() => setClearConfirmOpen(false)}
      />
    </div>
  );
}
