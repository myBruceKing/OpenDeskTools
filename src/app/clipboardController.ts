import type { ClipboardClient } from "./clipboardClient";
import {
  createClipboardLoadingState,
  createClipboardReadyViewModel,
  normalizeClipboardCommandError,
  toClipboardItemViewModel,
  type ClipboardCommandError,
  type ClipboardControllerState
} from "./clipboardModel";

function operationIssue(): ClipboardCommandError {
  return {
    code: "clipboard_operation_not_applied",
    message: "",
    retryable: true
  };
}

export class ClipboardController {
  private state = createClipboardLoadingState();
  private listeners = new Set<() => void>();
  private active = false;
  private session = 0;
  private request = 0;
  private itemRequests = new Map<string, number>();
  private errorOwner: string | null = null;

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
    const request = ++this.request;
    this.errorOwner = null;
    this.setState(createClipboardLoadingState());

    void this.client
      .getHistory({ scope: "all", search: null, limit: 100 })
      .then((result) => {
        if (!this.isCurrent(session, request)) {
          return;
        }
        this.errorOwner = null;
        this.setState({
          status: "ready",
          viewModel: createClipboardReadyViewModel(result),
          error: null,
          pendingItemIds: [],
          clearing: false
        });
      })
      .catch((error: unknown) => {
        if (!this.isCurrent(session, request)) {
          return;
        }
        const loading = createClipboardLoadingState();
        this.errorOwner = "load";
        this.setState({
          ...loading,
          status: "unavailable",
          viewModel: { ...loading.viewModel, monitoring: "unavailable" },
          error: normalizeClipboardCommandError(error)
        });
      });
  }

  stop() {
    this.active = false;
    this.session += 1;
    this.request += 1;
    this.itemRequests.clear();
    this.errorOwner = null;
  }

  async setFavorite(id: string, isFavorite: boolean) {
    if (!this.canMutateItem(id)) {
      return;
    }
    const session = this.session;
    const request = ++this.request;
    const errorOwner = `item:${id}`;
    this.itemRequests.set(id, request);
    this.markItemPending(id, true);

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
      }
    }
  }

  async deleteItem(id: string) {
    if (!this.canMutateItem(id)) {
      return;
    }
    const session = this.session;
    const request = ++this.request;
    const errorOwner = `item:${id}`;
    this.itemRequests.set(id, request);
    this.markItemPending(id, true);

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
    const request = ++this.request;
    const errorOwner = "clear";
    this.setState({ ...this.state, clearing: true });

    try {
      const result = await this.client.clearUnfavoriteHistory();
      if (!this.isCurrent(session, request)) {
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
      if (this.isCurrent(session, request)) {
        this.errorOwner = errorOwner;
        this.setState({
          ...this.state,
          clearing: false,
          error: normalizeClipboardCommandError(error)
        });
      }
    }
  }

  private canMutateItem(id: string) {
    return this.active
      && this.state.status === "ready"
      && !this.state.clearing
      && !this.state.pendingItemIds.includes(id)
      && this.state.viewModel.items.some((item) => item.id === id);
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

  private isCurrent(session: number, request: number) {
    return this.active && session === this.session && request === this.request;
  }

  private isCurrentItem(session: number, id: string, request: number) {
    return this.active
      && session === this.session
      && this.itemRequests.get(id) === request;
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
