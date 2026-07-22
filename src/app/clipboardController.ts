import type { ClipboardClient } from "./clipboardClient";
import {
  createClipboardLoadingState,
  createClipboardReadyViewModel,
  normalizeClipboardCommandError,
  toClipboardItemViewModel,
  type ClipboardCommandError,
  type ClipboardControllerState,
  type ClipboardHistoryResult,
  type ClipboardItemAction,
  type ClipboardMonitoringState,
  type ClipboardSettings,
  toClipboardSettingsViewModel
} from "./clipboardModel";

function operationIssue(): ClipboardCommandError {
  return {
    code: "clipboard_operation_not_applied",
    message: "",
    retryable: true
  };
}

type SubscriptionStatus = "pending" | "active" | "failed";

export class ClipboardController {
  private state = createClipboardLoadingState();
  private listeners = new Set<() => void>();
  private active = false;
  private session = 0;
  private operationRequest = 0;
  private actionRequest = 0;
  private surfaceRequest = 0;
  private historyRequest = 0;
  private itemRequests = new Map<string, number>();
  private errorOwner: string | null = null;
  private unlisten: (() => void) | null = null;
  private subscriptionStatus: SubscriptionStatus = "pending";
  private backendMonitoring: ClipboardHistoryResult["monitoring"] | null = null;
  private refreshInFlight = false;
  private refreshQueued = false;

  constructor(
    private readonly client: ClipboardClient,
    private surfaceActiveHint = false
  ) {
    this.state = createClipboardLoadingState(surfaceActiveHint);
  }

  getSnapshot = (): ClipboardControllerState => this.state;

  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  start() {
    this.stop();
    this.active = true;
    const session = this.session;
    this.errorOwner = null;
    this.subscriptionStatus = "pending";
    this.backendMonitoring = null;
    this.refreshInFlight = false;
    this.refreshQueued = false;
    this.setState(createClipboardLoadingState(this.surfaceActiveHint));

    void this.client
      .subscribe(() => {
        if (this.active && session === this.session) {
          this.queueRefresh(session);
        }
      })
      .then((unlisten) => {
        if (!this.active || session !== this.session) {
          unlisten();
          return;
        }
        this.unlisten = unlisten;
        this.subscriptionStatus = "active";
        this.setState({
          ...this.state,
          viewModel: {
            ...this.state.viewModel,
            monitoring: this.state.status === "unavailable"
              ? "unavailable"
              : this.effectiveMonitoring()
          },
          realtimeError: null
        });
        this.queueRefresh(session);
      })
      .catch(() => {
        if (!this.active || session !== this.session) {
          return;
        }
        this.subscriptionStatus = "failed";
        this.setState({
          ...this.state,
          viewModel: { ...this.state.viewModel, monitoring: "unavailable" },
          realtimeError: normalizeClipboardCommandError({
            code: "clipboard_subscription_unavailable"
          })
        });
      });

    this.queueRefresh(session);
  }

  stop() {
    this.active = false;
    this.session += 1;
    this.operationRequest += 1;
    this.actionRequest += 1;
    this.surfaceRequest += 1;
    this.historyRequest += 1;
    this.itemRequests.clear();
    this.errorOwner = null;
    this.unlisten?.();
    this.unlisten = null;
    this.refreshInFlight = false;
    this.refreshQueued = false;
  }

  async setFavorite(id: string, isFavorite: boolean) {
    if (!this.canMutateItem(id)) {
      return;
    }
    const session = this.session;
    const request = ++this.operationRequest;
    const errorOwner = `item:${id}`;
    this.itemRequests.set(id, request);
    this.markItemPending(id, true);
    if (this.refreshInFlight) {
      this.refreshQueued = true;
    }

    try {
      const item = await this.client.setFavorite(id, isFavorite);
      if (!this.isCurrentItem(session, id, request)) {
        return;
      }
      if (item.id !== id) {
        throw operationIssue();
      }
      this.setState({
        ...this.state,
        viewModel: {
          ...this.state.viewModel,
          items: this.state.viewModel.items.map((current) =>
            current.id === id ? toClipboardItemViewModel(item) : current
          )
        },
        error: this.consumeOwnedError(errorOwner)
      });
    } catch (error: unknown) {
      if (this.isCurrentItem(session, id, request)) {
        this.errorOwner = errorOwner;
        this.setState({ ...this.state, error: normalizeClipboardCommandError(error) });
      }
    } finally {
      if (this.isCurrentItem(session, id, request)) {
        this.itemRequests.delete(id);
        this.markItemPending(id, false);
        this.processQueuedRefresh(session);
      }
    }
  }

  async updateText(id: string, textContent: string, expectedRevision: number) {
    const current = this.state.viewModel.items.find((item) => item.id === id);
    if (
      !this.canMutateItem(id)
      || current?.kind !== "text"
      || current.revision !== expectedRevision
    ) {
      return false;
    }

    if (textContent.trim().length === 0) {
      this.setState({
        ...this.state,
        textEdit: {
          itemId: id,
          status: "error",
          message: "内容不能为空。",
          code: "clipboard_edit_empty",
          retryable: false
        }
      });
      return false;
    }

    const session = this.session;
    const request = ++this.operationRequest;
    this.itemRequests.set(id, request);
    this.setState({
      ...this.state,
      textEdit: {
        itemId: id,
        status: "pending",
        message: "正在保存…",
        code: null,
        retryable: false
      }
    });
    this.markItemPending(id, true);
    if (this.refreshInFlight) {
      this.refreshQueued = true;
    }

    let saved = false;
    try {
      const item = await this.client.updateText(id, textContent, expectedRevision);
      if (!this.isCurrentItem(session, id, request)) {
        return false;
      }
      if (item.id !== id || item.kind !== "text") {
        throw operationIssue();
      }
      saved = true;
      this.setState({
        ...this.state,
        viewModel: {
          ...this.state.viewModel,
          items: this.state.viewModel.items.map((candidate) =>
            candidate.id === id ? toClipboardItemViewModel(item) : candidate
          )
        },
        textEdit: {
          itemId: id,
          status: "success",
          message: "已保存。",
          code: null,
          retryable: false
        }
      });
    } catch (error: unknown) {
      if (this.isCurrentItem(session, id, request)) {
        const issue = normalizeClipboardCommandError(error);
        if (issue.code === "clipboard_revision_conflict") {
          this.refreshQueued = true;
        }
        this.setState({
          ...this.state,
          textEdit: {
            itemId: id,
            status: "error",
            message: issue.message,
            code: issue.code,
            retryable: issue.retryable
          }
        });
      }
    } finally {
      if (this.isCurrentItem(session, id, request)) {
        this.itemRequests.delete(id);
        this.markItemPending(id, false);
        this.processQueuedRefresh(session);
      }
    }
    return saved;
  }

  async copyItem(id: string) {
    await this.performItemAction("copy", id);
  }

  async inputItem(id: string) {
    await this.performItemAction("input", id);
  }

  async setMonitoring(enabled: boolean) {
    if (!this.active || this.state.monitoringPending || this.state.viewModel.monitoring === "unavailable") {
      return;
    }
    const session = this.session;
    this.setState({ ...this.state, monitoringPending: true, error: null });
    try {
      const monitoring = await this.client.setMonitoring(enabled);
      if (!this.active || session !== this.session) return;
      this.backendMonitoring = monitoring;
      this.setState({
        ...this.state,
        monitoringPending: false,
        viewModel: { ...this.state.viewModel, monitoring: this.effectiveMonitoring(monitoring) }
      });
      this.queueRefresh(session);
    } catch (error: unknown) {
      if (!this.active || session !== this.session) return;
      this.setState({
        ...this.state,
        monitoringPending: false,
        error: normalizeClipboardCommandError(error)
      });
    }
  }

  async updateSettings(settings: ClipboardSettings) {
    if (!this.active || this.state.settingsPending || !this.client.updateSettings) return false;
    const session = this.session;
    this.setState({ ...this.state, settingsPending: true, settingsMessage: null, error: null });
    try {
      const result = await this.client.updateSettings(settings);
      if (!this.active || session !== this.session) return false;
      this.setState({
        ...this.state,
        settingsPending: false,
        settingsMessage: result.removedCount > 0
          ? `设置已保存，已清理 ${result.removedCount} 条不再保留的历史记录。`
          : "设置已保存。",
        viewModel: { ...this.state.viewModel, settings: toClipboardSettingsViewModel(result.settings) }
      });
      this.queueRefresh(session);
      return true;
    } catch (error: unknown) {
      if (!this.active || session !== this.session) return false;
      this.setState({
        ...this.state,
        settingsPending: false,
        settingsMessage: null,
        error: normalizeClipboardCommandError(error)
      });
      return false;
    }
  }

  setSurfaceActiveHint(active: boolean) {
    this.surfaceActiveHint = active;
    if (active && !this.state.surfaceActive) {
      this.setState({ ...this.state, surfaceActive: true });
    }
  }

  async closeSurface() {
    if (this.active && !this.state.surfaceActive) {
      return true;
    }
    if (
      !this.active
      || !this.state.surfaceActive
      || this.state.surfaceClosing
      || this.state.itemAction?.status === "pending"
    ) {
      return false;
    }
    const session = this.session;
    const request = ++this.surfaceRequest;
    this.setState({
      ...this.state,
      surfaceClosing: true,
      surfaceError: null
    });

    try {
      const result = await this.client.closeSurface();
      if (!this.isCurrentSurface(session, request)) {
        return false;
      }
      this.setState({
        ...this.state,
        viewModel: {
          ...this.state.viewModel,
          actions: {
            ...this.state.viewModel.actions,
            canTypeIntoTarget: result.inputAvailable
          }
        },
        surfaceActive: false,
        surfaceClosing: false,
        surfaceError: null
      });
      return true;
    } catch (error: unknown) {
      if (!this.isCurrentSurface(session, request)) {
        return false;
      }
      const issue = normalizeClipboardCommandError(error);
      this.setState({
        ...this.state,
        viewModel: issue.code === "clipboard_target_unavailable"
          ? {
              ...this.state.viewModel,
              actions: {
                ...this.state.viewModel.actions,
                canTypeIntoTarget: false
              }
            }
          : this.state.viewModel,
        surfaceClosing: false,
        surfaceError: issue
      });
      return false;
    }
  }

  async deleteItem(id: string) {
    if (!this.canMutateItem(id)) {
      return;
    }
    const session = this.session;
    const request = ++this.operationRequest;
    const errorOwner = `item:${id}`;
    this.itemRequests.set(id, request);
    this.markItemPending(id, true);
    if (this.refreshInFlight) {
      this.refreshQueued = true;
    }

    try {
      const result = await this.client.deleteItem(id);
      if (!this.isCurrentItem(session, id, request)) {
        return;
      }
      if (!result.deleted) {
        throw operationIssue();
      }
      const nextItems = this.state.viewModel.items.filter((item) => item.id !== id);
      this.setState({
        ...this.state,
        viewModel: {
          ...this.state.viewModel,
          totalCount: Math.max(nextItems.length, this.state.viewModel.totalCount - 1),
          items: nextItems
        },
        itemAction: this.state.itemAction?.itemId === id ? null : this.state.itemAction,
        textEdit: this.state.textEdit?.itemId === id ? null : this.state.textEdit,
        error: this.consumeOwnedError(errorOwner)
      });
    } catch (error: unknown) {
      if (this.isCurrentItem(session, id, request)) {
        this.errorOwner = errorOwner;
        this.setState({ ...this.state, error: normalizeClipboardCommandError(error) });
      }
    } finally {
      if (this.isCurrentItem(session, id, request)) {
        this.itemRequests.delete(id);
        this.markItemPending(id, false);
        this.processQueuedRefresh(session);
      }
    }
  }

  async clearUnfavoriteHistory() {
    if (
      !this.active
      || this.state.status !== "ready"
      || this.state.clearing
      || this.state.pendingItemIds.length > 0
      || this.state.itemAction?.status === "pending"
    ) {
      return;
    }
    const session = this.session;
    const request = ++this.operationRequest;
    const errorOwner = "clear";
    this.setState({ ...this.state, clearing: true });
    if (this.refreshInFlight) {
      this.refreshQueued = true;
    }

    try {
      const result = await this.client.clearUnfavoriteHistory();
      if (!this.isCurrentOperation(session, request)) {
        return;
      }
      const nextItems = result.deletedCount > 0
        ? this.state.viewModel.items.filter((item) => item.favorite)
        : this.state.viewModel.items;
      this.setState({
        ...this.state,
        viewModel: {
          ...this.state.viewModel,
          totalCount: Math.max(
            nextItems.length,
            this.state.viewModel.totalCount - result.deletedCount
          ),
          items: nextItems
        },
        itemAction: this.state.itemAction
          && nextItems.some((item) => item.id === this.state.itemAction?.itemId)
          ? this.state.itemAction
          : null,
        clearing: false,
        error: this.consumeOwnedError(errorOwner)
      });
    } catch (error: unknown) {
      if (this.isCurrentOperation(session, request)) {
        this.errorOwner = errorOwner;
        this.setState({
          ...this.state,
          clearing: false,
          error: normalizeClipboardCommandError(error)
        });
      }
    } finally {
      if (this.isCurrentOperation(session, request)) {
        this.processQueuedRefresh(session);
      }
    }
  }

  private async performItemAction(action: ClipboardItemAction, id: string) {
    if (!this.canRunItemAction(action, id)) {
      return;
    }
    const session = this.session;
    const request = ++this.actionRequest;
    this.setState({
      ...this.state,
      itemAction: {
        action,
        itemId: id,
        status: "pending",
        message: action === "copy" ? "正在复制…" : "正在输入…",
        code: null,
        retryable: false
      },
      surfaceError: null
    });

    try {
      const result = action === "copy"
        ? await this.client.copyItem(id)
        : await this.client.inputItem(id);
      if (!this.isCurrentItemAction(session, request, action, id)) {
        return;
      }
      const message = action === "copy"
        ? "已复制到系统剪贴板。"
        : "已输入；该记录已保留在系统剪贴板。";
      const nextViewModel = this.state.viewModel.settings.historyReuseStrategy === "使用后移到最前"
        ? this.promoteItemInViewModel(id)
        : this.state.viewModel;
      this.setState({
        ...this.state,
        viewModel: action === "input"
          ? {
              ...nextViewModel,
              actions: {
                ...nextViewModel.actions,
                canTypeIntoTarget: false
              }
            }
          : nextViewModel,
        surfaceActive: action === "input" ? false : this.state.surfaceActive,
        itemAction: {
          action,
          itemId: id,
          status: "success",
          message,
          code: null,
          retryable: false
        }
      });
    } catch (error: unknown) {
      if (!this.isCurrentItemAction(session, request, action, id)) {
        return;
      }
      const issue = normalizeClipboardCommandError(error);
      this.setState({
        ...this.state,
        viewModel: action === "input"
          && (
            issue.code === "clipboard_target_unavailable"
            || issue.code === "clipboard_input_denied"
            || issue.code === "clipboard_input_cleanup_failed"
          )
          ? {
              ...this.state.viewModel,
              actions: {
                ...this.state.viewModel.actions,
                canTypeIntoTarget: false
              }
            }
          : this.state.viewModel,
        itemAction: {
          action,
          itemId: id,
          status: "error",
          message: issue.message,
          code: issue.code,
          retryable: issue.retryable
        }
      });
    }
  }

  private queueRefresh(session: number) {
    if (!this.active || session !== this.session) {
      return;
    }
    this.refreshQueued = true;
    this.processQueuedRefresh(session);
  }

  private processQueuedRefresh(session: number) {
    if (
      !this.active
      || session !== this.session
      || !this.refreshQueued
      || this.refreshInFlight
      || this.hasActiveMutation()
    ) {
      return;
    }
    this.refreshQueued = false;
    this.refreshInFlight = true;
    const request = ++this.historyRequest;

    void this.client
      .getHistory({ scope: "all", search: null, limit: 100 })
      .then((result) => {
        if (!this.isCurrentHistory(session, request)) {
          return;
        }
        if (this.refreshQueued || this.hasActiveMutation()) {
          this.refreshQueued = true;
          return;
        }
        this.backendMonitoring = result.monitoring;
        const viewModel = createClipboardReadyViewModel(result);
        const targetBecameAvailable = result.inputAvailable
          && !this.state.viewModel.actions.canTypeIntoTarget;
        this.setState({
          ...this.state,
          status: "ready",
          viewModel: {
            ...viewModel,
            monitoring: this.effectiveMonitoring(result.monitoring)
          },
          surfaceActive: result.surfaceActive,
          surfaceClosing: false,
          surfaceError: result.surfaceActive ? this.state.surfaceError : null,
          itemAction: this.state.itemAction
            && viewModel.items.some((item) => item.id === this.state.itemAction?.itemId)
            && !(targetBecameAvailable && this.state.itemAction.action === "input")
            ? this.state.itemAction
            : null,
          error: this.consumeOwnedError("history")
        });
      })
      .catch((error: unknown) => {
        if (!this.isCurrentHistory(session, request)) {
          return;
        }
        if (this.refreshQueued || this.hasActiveMutation()) {
          this.refreshQueued = true;
          return;
        }
        this.errorOwner = "history";
        const issue = normalizeClipboardCommandError(error);
        if (this.state.status === "ready") {
          this.setState({
            ...this.state,
            viewModel: { ...this.state.viewModel, monitoring: "unavailable" },
            error: issue
          });
        } else {
          const loading = createClipboardLoadingState(this.state.surfaceActive);
          this.setState({
            ...loading,
            status: "unavailable",
            viewModel: { ...loading.viewModel, monitoring: "unavailable" },
            error: issue,
            realtimeError: this.state.realtimeError
          });
        }
      })
      .finally(() => {
        if (!this.isCurrentHistory(session, request)) {
          return;
        }
        this.refreshInFlight = false;
        this.processQueuedRefresh(session);
      });
  }

  private effectiveMonitoring(
    backendMonitoring = this.backendMonitoring
  ): ClipboardMonitoringState {
    if (this.subscriptionStatus === "failed" || backendMonitoring === "unavailable") {
      return "unavailable";
    }
    if (this.subscriptionStatus === "active" && backendMonitoring === "running") {
      return "running";
    }
    return "paused";
  }

  private promoteItemInViewModel(id: string) {
    const item = this.state.viewModel.items.find((candidate) => candidate.id === id);
    if (!item) {
      return this.state.viewModel;
    }
    return {
      ...this.state.viewModel,
      items: [item, ...this.state.viewModel.items.filter((candidate) => candidate.id !== id)]
    };
  }

  private canMutateItem(id: string) {
    return this.active
      && this.state.status === "ready"
      && !this.state.clearing
      && !this.state.pendingItemIds.includes(id)
      && !(this.state.itemAction?.status === "pending" && this.state.itemAction.itemId === id)
      && this.state.viewModel.items.some((item) => item.id === id);
  }

  private canRunItemAction(action: ClipboardItemAction, id: string) {
    const availability = action === "copy"
      ? this.state.viewModel.actions.canCopy
      : this.state.viewModel.actions.canTypeIntoTarget;
    return this.active
      && this.state.status === "ready"
      && !this.state.clearing
      && this.state.itemAction?.status !== "pending"
      && !this.state.pendingItemIds.includes(id)
      && availability
      && this.state.viewModel.items.some((item) => item.id === id);
  }

  private hasActiveMutation() {
    return this.state.clearing || this.state.pendingItemIds.length > 0;
  }

  private markItemPending(id: string, pending: boolean) {
    const ids = new Set(this.state.pendingItemIds);
    if (pending) {
      ids.add(id);
    } else {
      ids.delete(id);
    }
    this.setState({ ...this.state, pendingItemIds: [...ids] });
  }

  private isCurrentOperation(session: number, request: number) {
    return this.active && session === this.session && request === this.operationRequest;
  }

  private isCurrentItem(session: number, id: string, request: number) {
    return this.active
      && session === this.session
      && this.itemRequests.get(id) === request;
  }

  private isCurrentHistory(session: number, request: number) {
    return this.active
      && session === this.session
      && request === this.historyRequest;
  }

  private isCurrentItemAction(
    session: number,
    request: number,
    action: ClipboardItemAction,
    id: string
  ) {
    return this.active
      && session === this.session
      && request === this.actionRequest
      && this.state.itemAction?.status === "pending"
      && this.state.itemAction.action === action
      && this.state.itemAction.itemId === id;
  }

  private isCurrentSurface(session: number, request: number) {
    return this.active
      && session === this.session
      && request === this.surfaceRequest
      && this.state.surfaceClosing;
  }

  private consumeOwnedError(owner: string) {
    if (this.errorOwner !== owner) {
      return this.state.error;
    }
    this.errorOwner = null;
    return null;
  }

  private setState(state: ClipboardControllerState) {
    this.state = state;
    for (const listener of this.listeners) {
      listener();
    }
  }
}
