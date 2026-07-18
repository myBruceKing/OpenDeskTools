import type { ClipboardClient } from "./clipboardClient";
import {
  createClipboardLoadingState,
  createClipboardReadyViewModel,
  normalizeClipboardCommandError,
  toClipboardItemViewModel,
  type ClipboardCommandError,
  type ClipboardControllerState,
  type ClipboardHistoryResult,
  type ClipboardMonitoringState
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
  private historyRequest = 0;
  private itemRequests = new Map<string, number>();
  private errorOwner: string | null = null;
  private unlisten: (() => void) | null = null;
  private subscriptionStatus: SubscriptionStatus = "pending";
  private backendMonitoring: ClipboardHistoryResult["monitoring"] | null = null;
  private refreshInFlight = false;
  private refreshQueued = false;

  constructor(private readonly client: ClipboardClient) {}

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
    this.setState(createClipboardLoadingState());

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
        this.setState({
          ...this.state,
          status: "ready",
          viewModel: {
            ...createClipboardReadyViewModel(result),
            monitoring: this.effectiveMonitoring(result.monitoring)
          },
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
          const loading = createClipboardLoadingState();
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

  private canMutateItem(id: string) {
    return this.active
      && this.state.status === "ready"
      && !this.state.clearing
      && !this.state.pendingItemIds.includes(id)
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
