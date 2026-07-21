import { getCurrentWindow } from "@tauri-apps/api/window";
import { Dismiss20Regular } from "@fluentui/react-icons";
import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type MouseEvent
} from "react";
import type {
  ClipboardPreviewDebugEvent,
  ClipboardPreviewHoverChange
} from "../../app/clipboardClient";
import type {
  ClipboardControllerState,
  ClipboardFilter,
  ClipboardItemViewModel
} from "../../app/clipboardModel";
import { ConfirmDialog } from "../../components/primitives/Dialog";
import {
  ClipboardHistoryFilter,
  clipboardHistoryFilterLabel
} from "../../components/patterns/ClipboardHistoryControls";
import {
  ClipboardHistoryContextMenu,
  ClipboardHistoryRowContent,
  type ClipboardHistoryMenuCloseReason
} from "../../components/patterns/ClipboardHistoryItem";
import { type LoadClipboardSourceIcon } from "../../components/patterns/SourceAppIcon";
import styles from "./ClipboardSurface.module.css";

type ClipboardSurfaceProps = {
  state: ClipboardControllerState;
  loadSourceIcon: LoadClipboardSourceIcon;
  onCopy: (id: string) => void;
  onInput: (id: string) => void;
  onClose: () => Promise<boolean>;
  onSetFavorite: (id: string, isFavorite: boolean) => void;
  onDelete: (id: string) => void;
  onOpenPreview: (id: string) => Promise<void>;
  onClosePreview: () => Promise<void>;
  onSubscribePreviewHover: (
    listener: (change: ClipboardPreviewHoverChange) => void
  ) => Promise<() => void>;
  onTracePreviewDebug: (
    event: ClipboardPreviewDebugEvent,
    recordId?: string | null
  ) => Promise<void>;
};

function optionId(id: string) {
  return `clipboard-surface-option-${id.replace(/[^a-zA-Z0-9_-]/g, "_")}`;
}

function previewFailureMessage(action: "open" | "close" | "subscribe", error: unknown) {
  const detail = error instanceof Error && error.message.trim().length > 0
    ? error.message.trim()
    : typeof error === "string" && error.trim().length > 0
      ? error.trim()
      : "未知错误";
  const actionLabel = action === "open" ? "打开" : action === "close" ? "关闭" : "连接";
  return `${actionLabel}预览窗口失败：${detail}`;
}

export function ClipboardSurface({
  state,
  loadSourceIcon,
  onCopy,
  onInput,
  onClose,
  onSetFavorite,
  onDelete,
  onOpenPreview,
  onClosePreview,
  onSubscribePreviewHover,
  onTracePreviewDebug
}: ClipboardSurfaceProps) {
  const [filter, setFilter] = useState<ClipboardFilter>("all");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [previewId, setPreviewId] = useState<string | null>(null);
  const [menu, setMenu] = useState<{
    id: string;
    point: { x: number; y: number };
    keyboardOpened: boolean;
  } | null>(null);
  const [deleteId, setDeleteId] = useState<string | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [visibleItemAction, setVisibleItemAction] = useState(state.surfaceActive ? state.itemAction : null);
  const rowRefs = useRef(new Map<string, HTMLDivElement>());
  const previewCloseTimer = useRef<number | null>(null);
  const previewIdRef = useRef<string | null>(null);
  const previewCommandGeneration = useRef(0);
  const previewCommandTail = useRef<Promise<void>>(Promise.resolve());
  const wasSurfaceActive = useRef(state.surfaceActive);
  const lastPresentedItemAction = useRef<ClipboardControllerState["itemAction"]>(null);
  const items = useMemo(() => state.viewModel.items.filter((item) => {
    if (filter === "text") return item.displayCategory === "text";
    if (filter === "image") return item.displayCategory === "image";
    if (filter === "favorite") return item.favorite;
    return true;
  }), [filter, state.viewModel.items]);
  const selectedItem = items.find((item) => item.id === selectedId) ?? items[0] ?? null;
  const menuItem = menu ? items.find((item) => item.id === menu.id) ?? null : null;
  const deleteItem = deleteId ? state.viewModel.items.find((item) => item.id === deleteId) ?? null : null;

  useEffect(() => {
    if (!selectedId && items[0]) setSelectedId(items[0].id);
    if (selectedId && !items.some((item) => item.id === selectedId)) {
      setSelectedId(items[0]?.id ?? null);
    }
  }, [items, selectedId]);

  useEffect(() => {
    const wasActive = wasSurfaceActive.current;
    wasSurfaceActive.current = state.surfaceActive;
    if (!state.surfaceActive) {
      closePreviewNow();
      setPreviewError(null);
      setMenu(null);
      setDeleteId(null);
      setVisibleItemAction(null);
      return;
    }
    if (!wasActive) {
      if (previewCloseTimer.current !== null) window.clearTimeout(previewCloseTimer.current);
      previewCloseTimer.current = null;
      setFilter("all");
      setSelectedId(state.viewModel.items[0]?.id ?? null);
      previewIdRef.current = null;
      setPreviewId(null);
      setPreviewError(null);
      setMenu(null);
      setDeleteId(null);
      setVisibleItemAction(null);
    }
  }, [state.surfaceActive, state.viewModel.items]);

  useEffect(() => {
    const action = state.itemAction;
    if (!state.surfaceActive || !action || action === lastPresentedItemAction.current) return undefined;
    lastPresentedItemAction.current = action;
    setVisibleItemAction(action);
    if (action.status === "pending") return undefined;
    const timer = window.setTimeout(
      () => setVisibleItemAction((current) => current === action ? null : current),
      action.status === "error" ? 5000 : 2600
    );
    return () => window.clearTimeout(timer);
  }, [state.itemAction, state.surfaceActive]);

  useEffect(() => {
    if (menu && !items.some((item) => item.id === menu.id)) setMenu(null);
    if (deleteId && !state.viewModel.items.some((item) => item.id === deleteId)) setDeleteId(null);
    if (previewId && !items.some((item) => item.id === previewId)) closePreviewNow();
  }, [deleteId, items, menu, state.viewModel.items]);

  const tracePreview = (event: ClipboardPreviewDebugEvent, recordId = previewIdRef.current) => {
    void onTracePreviewDebug(event, recordId).catch((error) => {
      console.error("Failed to write clipboard preview debug trace", error);
    });
  };

  const queuePreviewCommand = (
    nextPreviewId: string | null,
    traceRecordId = nextPreviewId
  ) => {
    const generation = ++previewCommandGeneration.current;
    setPreviewError(null);
    const run = async () => {
      tracePreview(nextPreviewId === null ? "close_requested" : "open_requested", traceRecordId);
      try {
        if (nextPreviewId === null) await onClosePreview();
        else await onOpenPreview(nextPreviewId);
        tracePreview(nextPreviewId === null ? "close_resolved" : "open_resolved", traceRecordId);
        if (previewCommandGeneration.current === generation) setPreviewError(null);
      } catch (error) {
        tracePreview(nextPreviewId === null ? "close_failed" : "open_failed", traceRecordId);
        if (previewCommandGeneration.current !== generation) return;
        if (nextPreviewId !== null && previewIdRef.current === nextPreviewId) {
          previewIdRef.current = null;
          setPreviewId(null);
        }
        setPreviewError(previewFailureMessage(nextPreviewId === null ? "close" : "open", error));
      }
    };
    previewCommandTail.current = previewCommandTail.current.then(run, run);
  };

  const closePreviewNow = () => {
    if (previewCloseTimer.current !== null) window.clearTimeout(previewCloseTimer.current);
    previewCloseTimer.current = null;
    const closingPreviewId = previewIdRef.current;
    previewIdRef.current = null;
    setPreviewId(null);
    if (closingPreviewId !== null) queuePreviewCommand(null, closingPreviewId);
  };
  const schedulePreviewClose = () => {
    if (previewCloseTimer.current !== null) window.clearTimeout(previewCloseTimer.current);
    tracePreview("close_scheduled");
    previewCloseTimer.current = window.setTimeout(() => {
      tracePreview("close_fired");
      closePreviewNow();
    }, 240);
  };
  const keepPreviewOpen = () => {
    if (previewCloseTimer.current !== null) {
      window.clearTimeout(previewCloseTimer.current);
      tracePreview("close_canceled");
    }
    previewCloseTimer.current = null;
  };

  const openPreview = (id: string) => {
    keepPreviewOpen();
    if (previewIdRef.current === id) return;
    previewIdRef.current = id;
    setPreviewId(id);
    queuePreviewCommand(id);
  };

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void (async () => {
      try {
        const cleanup = await onSubscribePreviewHover((change) => {
          if (disposed || change.recordId !== previewIdRef.current) return;
          if (change.inside) {
            tracePreview("hover_inside", change.recordId);
            keepPreviewOpen();
          } else {
            tracePreview("hover_outside", change.recordId);
            schedulePreviewClose();
          }
        });
        if (disposed) cleanup();
        else unlisten = cleanup;
      } catch (error) {
        if (!disposed) setPreviewError(previewFailureMessage("subscribe", error));
      }
    })();
    return () => {
      disposed = true;
      if (previewCloseTimer.current !== null) window.clearTimeout(previewCloseTimer.current);
      previewCloseTimer.current = null;
      unlisten?.();
    };
  }, [onSubscribePreviewHover]);

  useEffect(() => {
    if (!previewId) return undefined;
    const closeOnViewportChange = () => closePreviewNow();
    const ignoreWindowBlur = () => tracePreview("window_blur_ignored", previewId);
    window.addEventListener("resize", closeOnViewportChange);
    window.addEventListener("blur", ignoreWindowBlur);
    return () => {
      window.removeEventListener("resize", closeOnViewportChange);
      window.removeEventListener("blur", ignoreWindowBlur);
    };
  }, [previewId]);

  const focusItem = (id: string) => {
    setSelectedId(id);
    window.requestAnimationFrame(() => rowRefs.current.get(id)?.focus());
  };

  const moveSelection = (direction: number) => {
    if (!selectedItem) return;
    const index = items.findIndex((item) => item.id === selectedItem.id);
    const next = items[Math.min(items.length - 1, Math.max(0, index + direction))];
    if (next) focusItem(next.id);
  };

  const requestDelete = (id: string) => {
    closePreviewNow();
    setMenu(null);
    setDeleteId(id);
  };

  const resetTransientLayers = () => {
    closePreviewNow();
    setMenu(null);
    setDeleteId(null);
  };

  const closeMenu = (reason: ClipboardHistoryMenuCloseReason, id = menu?.id) => {
    setMenu(null);
    if (reason === "keyboard" && id) focusItem(id);
  };

  const closeTopLayerOrSurface = () => {
    if (menu) {
      const id = menu.id;
      closeMenu("keyboard", id);
      return;
    }
    if (previewId) {
      closePreviewNow();
      return;
    }
    void onClose();
  };

  useEffect(() => {
    const handleEscape = (event: globalThis.KeyboardEvent) => {
      if (
        event.key !== "Escape"
        || event.defaultPrevented
        || event.repeat
        || event.ctrlKey
        || event.altKey
        || event.metaKey
        || document.querySelector('[role="dialog"]')
      ) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      closeTopLayerOrSurface();
    };
    document.addEventListener("keydown", handleEscape);
    return () => document.removeEventListener("keydown", handleEscape);
  });

  const openContextMenu = (item: ClipboardItemViewModel, event: MouseEvent) => {
    event.preventDefault();
    setSelectedId(item.id);
    closePreviewNow();
    setMenu({ id: item.id, point: { x: event.clientX, y: event.clientY }, keyboardOpened: false });
  };

  const changeFilter = (nextFilter: ClipboardFilter) => {
    setFilter(nextFilter);
    closePreviewNow();
    setMenu(null);
  };

  return (
    <section
      className={styles.surface}
      aria-label="剪贴板快捷面板"
      data-window-border-layer="true"
    >
      <header
        className={styles.titlebar}
        aria-label="剪贴板筛选与窗口操作"
        onMouseDown={(event) => {
          if (event.button === 0 && !(event.target as HTMLElement).closest("button")) {
            void getCurrentWindow().startDragging().catch(() => undefined);
          }
        }}
      >
        <ClipboardHistoryFilter value={filter} onChange={changeFilter} />
        <button
          type="button"
          aria-label="关闭剪贴板面板"
          disabled={state.surfaceClosing}
          onClick={() => {
            resetTransientLayers();
            void onClose();
          }}
        >
          <Dismiss20Regular aria-hidden="true" />
        </button>
      </header>

      <div
        className={styles.history}
        role="listbox"
        aria-label="剪贴板历史"
        aria-activedescendant={selectedItem ? optionId(selectedItem.id) : undefined}
        onScroll={() => {
          closePreviewNow();
          setMenu(null);
        }}
      >
        {state.status === "loading" ? (
          <div className={styles.empty} role="status">正在加载剪贴板历史…</div>
        ) : state.status === "unavailable" ? (
          <div className={styles.empty} role="alert">{state.error?.message ?? "剪贴板历史不可用"}</div>
        ) : items.length === 0 ? (
          <div className={styles.empty}>{filter === "all" ? "暂无剪贴板历史" : `暂无${clipboardHistoryFilterLabel(filter)}内容`}</div>
        ) : items.map((item) => {
          const selected = item.id === selectedItem?.id;
          const pending = state.pendingItemIds.includes(item.id)
            || (state.itemAction?.status === "pending" && state.itemAction.itemId === item.id);
          return (
            <div
              ref={(node) => {
                if (node) rowRefs.current.set(item.id, node);
                else rowRefs.current.delete(item.id);
              }}
              id={optionId(item.id)}
              className={[
                styles.row,
                selected ? styles.rowSelected : "",
                !state.viewModel.actions.canTypeIntoTarget ? styles.rowBrowseOnly : ""
              ].filter(Boolean).join(" ")}
              role="option"
              aria-selected={selected}
              aria-busy={pending || undefined}
              aria-describedby={!state.viewModel.actions.canTypeIntoTarget ? "clipboard-surface-browse-notice" : undefined}
              title={!state.viewModel.actions.canTypeIntoTarget ? "当前仅可浏览；复制后可在目标应用中手动粘贴" : undefined}
              tabIndex={selected ? 0 : -1}
              key={item.id}
              onFocus={(event) => {
                setSelectedId(item.id);
                setMenu(null);
                openPreview(item.id);
              }}
              onBlur={(event) => {
                const relatedTarget = event.relatedTarget as Node | null;
                if (!relatedTarget || !event.currentTarget.contains(relatedTarget)) schedulePreviewClose();
              }}
              onMouseEnter={() => {
                openPreview(item.id);
              }}
              onMouseLeave={schedulePreviewClose}
              onMouseDown={(event) => {
                if (event.detail > 1) event.preventDefault();
              }}
              onClick={() => {
                setSelectedId(item.id);
                setMenu(null);
              }}
              onDoubleClick={(event) => {
                event.preventDefault();
                if (state.viewModel.actions.canTypeIntoTarget && !pending) onInput(item.id);
              }}
              onContextMenu={(event) => openContextMenu(item, event)}
              onKeyDown={(event: KeyboardEvent<HTMLDivElement>) => {
                if (event.key === "ArrowDown") {
                  event.preventDefault();
                  moveSelection(1);
                } else if (event.key === "ArrowUp") {
                  event.preventDefault();
                  moveSelection(-1);
                } else if (event.key === "Home" && items[0]) {
                  event.preventDefault();
                  focusItem(items[0].id);
                } else if (event.key === "End" && items[items.length - 1]) {
                  event.preventDefault();
                  focusItem(items[items.length - 1].id);
                } else if (event.key === "Enter" && state.viewModel.actions.canTypeIntoTarget && !pending) {
                  event.preventDefault();
                  onInput(item.id);
                } else if (event.key.toLocaleLowerCase() === "c" && event.ctrlKey && !pending) {
                  event.preventDefault();
                  onCopy(item.id);
                } else if (event.key === "Delete" && !pending) {
                  event.preventDefault();
                  requestDelete(item.id);
                } else if (event.key === "ContextMenu" || (event.key === "F10" && event.shiftKey)) {
                  event.preventDefault();
                  const rect = event.currentTarget.getBoundingClientRect();
                  setMenu({
                    id: item.id,
                    point: { x: rect.right - 150, y: rect.top + 24 },
                    keyboardOpened: true
                  });
                } else if (event.key === "Escape") {
                  event.preventDefault();
                  event.stopPropagation();
                  closeTopLayerOrSurface();
                }
              }}
            >
              <ClipboardHistoryRowContent
                className={styles.rowContent}
                item={item}
                loadSourceIcon={loadSourceIcon}
                favoriteDisabled={pending}
                deleteDisabled={pending}
                onToggleFavorite={() => onSetFavorite(item.id, !item.favorite)}
                onDelete={() => requestDelete(item.id)}
              />
            </div>
          );
        })}
      </div>

      {previewError && (
        <div className={styles.surfaceStatus} role="alert">
          {previewError}
        </div>
      )}

      {!previewError && visibleItemAction && (
        <div className={styles.surfaceStatus} role={visibleItemAction.status === "error" ? "alert" : "status"}>
          {visibleItemAction.message}
        </div>
      )}

      {!state.viewModel.actions.canTypeIntoTarget && (
        <span className={styles.srOnly} id="clipboard-surface-browse-notice">
          当前仅可浏览；复制后可在目标应用中手动粘贴。
        </span>
      )}

      {(state.surfaceError || state.surfaceClosing || !state.viewModel.actions.canTypeIntoTarget) && (
        <div
          className={[styles.surfaceMessage, state.surfaceError ? styles.surfaceMessageError : ""].filter(Boolean).join(" ")}
          role={state.surfaceError ? "alert" : "status"}
        >
          {state.surfaceError?.message
            ?? (state.surfaceClosing ? "正在关闭剪贴板面板…" : "仅浏览：复制后在目标应用中手动粘贴")}
        </div>
      )}

      {menu && menuItem && (
        <ClipboardHistoryContextMenu
          item={menuItem}
          point={menu.point}
          initialFocus={menu.keyboardOpened}
          onCopy={() => { onCopy(menuItem.id); setMenu(null); }}
          onToggleFavorite={() => { onSetFavorite(menuItem.id, !menuItem.favorite); setMenu(null); }}
          onDelete={() => requestDelete(menuItem.id)}
          onClose={(reason) => closeMenu(reason, menuItem.id)}
        />
      )}
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
    </section>
  );
}
