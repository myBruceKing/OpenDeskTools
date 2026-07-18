import {
  applyThemePatch,
  normalizeThemeCommandError,
  type ThemeCommandError,
  type ThemeControllerState,
  type ThemePatch,
  type ThemeSnapshot
} from "./themeModel";
import type { ThemeClient } from "./themeClient";

type PendingUpdate = {
  patch: ThemePatch;
  complete: () => void;
};

const INITIAL_STATE: ThemeControllerState = {
  status: "loading",
  confirmed: null,
  current: null,
  saving: false,
  error: null,
  warning: null
};

export class ThemeController {
  private state: ThemeControllerState = INITIAL_STATE;
  private listeners = new Set<() => void>();
  private pending: PendingUpdate[] = [];
  private processingSession: number | null = null;
  private active = false;
  private session = 0;
  private unlisten: (() => void) | null = null;

  constructor(private readonly client: ThemeClient) {}

  getSnapshot = (): ThemeControllerState => this.state;

  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  start() {
    this.stop();
    this.active = true;
    const session = this.session;
    this.setState({ ...INITIAL_STATE });

    void this.client
      .subscribe((snapshot) => {
        if (this.active && session === this.session) {
          this.acceptSnapshot(snapshot);
        }
      })
      .then((unlisten) => {
        if (this.active && session === this.session) {
          this.unlisten = unlisten;
        } else {
          unlisten();
        }
      })
      .catch((error: unknown) => {
        if (this.active && session === this.session) {
          const issue = normalizeThemeCommandError(error);
          this.setState({
            ...this.state,
            warning: { code: issue.code, message: "主题已加载，但无法接收其他窗口的实时主题更新。" }
          });
        }
      });

    void this.client
      .get()
      .then((snapshot) => {
        if (!this.active || session !== this.session) {
          return;
        }
        this.acceptSnapshot(snapshot, true);
      })
      .catch((error: unknown) => {
        if (!this.active || session !== this.session) {
          return;
        }
        if (this.state.confirmed !== null) {
          const issue = normalizeThemeCommandError(error);
          this.setState({
            ...this.state,
            status: "ready",
            warning: { code: issue.code, message: "已从实时同步恢复主题，但初始主题读取失败。" }
          });
          return;
        }
        this.setState({
          ...this.state,
          status: "unavailable",
          error: normalizeThemeCommandError(error),
          saving: false
        });
      });
  }

  stop() {
    this.active = false;
    this.session += 1;
    this.unlisten?.();
    this.unlisten = null;
    this.processingSession = null;
    for (const item of this.pending.splice(0)) {
      item.complete();
    }
  }

  update(patch: ThemePatch): Promise<void> {
    if (!this.active) {
      return Promise.resolve();
    }
    if (this.state.status !== "ready" || this.state.confirmed === null) {
      const error: ThemeCommandError = {
        code: "theme_unavailable",
        message: "主题服务当前不可用。",
        field: null,
        retryable: true,
        applied: false
      };
      this.setState({ ...this.state, error });
      return Promise.resolve();
    }

    return new Promise((complete) => {
      this.pending.push({ patch, complete });
      this.setState({
        ...this.state,
        current: this.computeCurrent(),
        saving: true,
        error: null,
        warning: null
      });
      void this.processQueue();
    });
  }

  private async processQueue() {
    if (this.processingSession === this.session || this.pending.length === 0) {
      return;
    }

    const session = this.session;
    this.processingSession = session;
    const item = this.pending[0];
    const expectedRevision = this.state.confirmed?.revision;

    if (expectedRevision === undefined) {
      this.pending.shift();
      item.complete();
      this.processingSession = null;
      return;
    }

    try {
      const result = await this.client.update(expectedRevision, item.patch);
      if (!this.active || session !== this.session) {
        return;
      }
      this.pending.shift();
      this.acceptSnapshot(result.snapshot, true, false);
      this.setState({
        ...this.state,
        warning: result.broadcastWarning,
        error: null
      });
    } catch (error: unknown) {
      if (!this.active || session !== this.session) {
        return;
      }
      this.pending.shift();
      let issue = normalizeThemeCommandError(error);

      if (issue.applied || issue.code === "theme_revision_conflict") {
        try {
          const snapshot = await this.client.get();
          if (!this.active || session !== this.session) {
            return;
          }
          this.acceptSnapshot(snapshot, true, false);
        } catch (refreshError: unknown) {
          if (!this.active || session !== this.session) {
            return;
          }
          issue = normalizeThemeCommandError(refreshError);
          this.setState({
            ...this.state,
            status: "unavailable",
            current: null,
            saving: false
          });
          for (const pending of this.pending.splice(0)) {
            pending.complete();
          }
        }
      }

      this.setState({
        ...this.state,
        current: this.state.status === "unavailable" ? null : this.computeCurrent(),
        error: issue,
        warning: null
      });
    } finally {
      item.complete();
      if (!this.active || session !== this.session) {
        return;
      }
      this.processingSession = null;
      this.setState({
        ...this.state,
        current: this.state.status === "unavailable" ? null : this.computeCurrent(),
        saving: this.state.status === "unavailable" ? false : this.pending.length > 0
      });
      if (this.state.status === "ready") {
        void this.processQueue();
      }
    }
  }

  private acceptSnapshot(snapshot: ThemeSnapshot, forceReady = false, emit = true) {
    const confirmed = this.state.confirmed;
    if (confirmed !== null && snapshot.revision <= confirmed.revision) {
      if (forceReady && this.state.status !== "ready") {
        this.setState({ ...this.state, status: "ready" });
      }
      return;
    }

    this.state = {
      ...this.state,
      status: "ready",
      confirmed: snapshot,
      current: this.computeCurrent(snapshot)
    };
    if (emit) {
      this.emit();
    }
  }

  private computeCurrent(confirmed = this.state.confirmed): ThemeSnapshot | null {
    if (confirmed === null) {
      return null;
    }
    return this.pending.reduce(
      (current, pending) => applyThemePatch(current, pending.patch),
      confirmed
    );
  }

  private setState(state: ThemeControllerState) {
    this.state = state;
    this.emit();
  }

  private emit() {
    for (const listener of this.listeners) {
      listener();
    }
  }
}
