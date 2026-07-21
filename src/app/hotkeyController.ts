import type { HotkeyClient } from "./hotkeyClient";
import {
  normalizeHotkeyCommandError,
  type GlobalHotkeyId,
  type HotkeyControllerState,
  type HotkeyEditorState,
  type HotkeySnapshot
} from "./hotkeyModel";

const INITIAL_STATE: HotkeyControllerState = {
  status: "loading",
  snapshot: null,
  editor: null,
  error: null,
  systemHotkeyNotice: null
};

export function appendHotkeyToken(binding: string, token: string) {
  const normalizedToken = token.trim();
  if (normalizedToken.length === 0) {
    return binding.trim();
  }
  return [...binding.trim().split(/\s+/).filter(Boolean), normalizedToken].join(" ");
}

export function canSaveHotkeyEditor(state: HotkeyControllerState) {
  const editor = state.editor;
  if (
    state.status !== "ready" ||
    editor === null ||
    editor.saving ||
    editor.classificationStatus !== "ready" ||
    editor.classification === null ||
    editor.binding.trim().length === 0
  ) {
    return false;
  }

  if (editor.classification.classification === "ordinary") {
    return true;
  }
  return (
    editor.classification.classification === "system_reserved" &&
    editor.classification.forceOverrideAllowed &&
    editor.forceOverrideSystem
  );
}

function forceOverrideRequested(editor: HotkeyEditorState) {
  return editor.classification?.classification === "system_reserved"
    && editor.classification.forceOverrideAllowed
    && editor.forceOverrideSystem;
}

function confirmSavedHotkey(
  snapshot: HotkeySnapshot,
  editor: HotkeyEditorState
) {
  const action = snapshot.actions.find((candidate) => candidate.actionId === editor.actionId);
  const requestedBinding = editor.binding.trim().replace(/\s+/g, " ");
  const requestedForceOverride = forceOverrideRequested(editor);

  if (!action) {
    return {
      code: "hotkey_update_not_applied",
      message: "保存未生效；快捷键服务未返回该功能的配置，请重试。",
      actualRevision: snapshot.revision
    };
  }

  if (action.binding !== requestedBinding) {
    return {
      code: "hotkey_update_not_applied",
      message: `保存未生效；快捷键服务当前仍返回 ${action.binding}，请重试。`,
      actualRevision: snapshot.revision
    };
  }

  if (action.forceOverrideSystem !== requestedForceOverride) {
    return {
      code: "hotkey_update_not_applied",
      message: `保存未生效；快捷键服务未确认 ${action.binding} 的强制覆盖授权，请重试。`,
      actualRevision: snapshot.revision
    };
  }

  if (
    requestedForceOverride
    && action.binding === "Win+V"
    && action.runtimeBackend === null
  ) {
    return {
      code: "hotkey_saved_not_active",
      message: "配置已保存为 Win+V，但快捷键服务未确认实际运行后端，请重试或重启应用后检查状态。",
      actualRevision: snapshot.revision
    };
  }

  if (
    action.actionAvailable
    && action.configuredEnabled
    && action.runtimeState !== "registered"
  ) {
    return {
      code: "hotkey_saved_not_active",
      message: `配置已保存为 ${action.binding}，但当前未生效${action.detail ? `：${action.detail}` : "。"}`,
      actualRevision: snapshot.revision
    };
  }

  return null;
}

export class HotkeyController {
  private state: HotkeyControllerState = INITIAL_STATE;
  private listeners = new Set<() => void>();
  private active = false;
  private session = 0;
  private classificationRequest = 0;

  constructor(private readonly client: HotkeyClient) {}

  getSnapshot = () => this.state;

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
      .getSnapshot()
      .then((snapshot) => {
        if (!this.active || session !== this.session) {
          return;
        }
        this.setState({
          status: "ready",
          snapshot,
          editor: null,
          error: null,
          systemHotkeyNotice: this.state.systemHotkeyNotice
        });
      })
      .catch((error: unknown) => {
        if (!this.active || session !== this.session) {
          return;
        }
        this.setState({
          status: "unavailable",
          snapshot: null,
          editor: null,
          error: normalizeHotkeyCommandError(error),
          systemHotkeyNotice: null
        });
      });
  }

  stop() {
    this.active = false;
    this.session += 1;
    this.classificationRequest += 1;
  }

  openEditor(actionId: GlobalHotkeyId) {
    if (!this.active || this.state.status !== "ready" || this.state.snapshot === null) {
      return;
    }
    const action = this.state.snapshot.actions.find((candidate) => candidate.actionId === actionId);
    if (!action) {
      return;
    }

    const editor: HotkeyEditorState = {
      actionId,
      actionAvailable: action.actionAvailable,
      binding: action.binding,
      inputDirty: false,
      classificationStatus: "loading",
      classification: null,
      forceOverrideSystem: action.forceOverrideSystem,
      saving: false,
      error: null
    };
    this.setState({ ...this.state, editor });
    this.classifyBinding(editor.binding, actionId);
  }

  closeEditor() {
    if (this.state.editor?.saving) {
      return;
    }
    this.classificationRequest += 1;
    this.setState({ ...this.state, editor: null });
  }

  setBinding(binding: string) {
    const editor = this.state.editor;
    if (!this.active || editor === null || editor.saving) {
      return;
    }

    this.setState({
      ...this.state,
      editor: {
        ...editor,
        binding,
        inputDirty: true,
        classificationStatus: "loading",
        classification: null,
        forceOverrideSystem: false,
        error: null
      }
    });
    this.classifyBinding(binding, editor.actionId);
  }

  appendBindingToken(token: string) {
    const editor = this.state.editor;
    if (!this.active || editor === null || editor.saving) {
      return;
    }
    this.setBinding(editor.inputDirty ? appendHotkeyToken(editor.binding, token) : token.trim());
  }

  setForceOverrideSystem(forceOverrideSystem: boolean) {
    const editor = this.state.editor;
    if (
      !this.active ||
      editor === null ||
      editor.saving ||
      editor.classification?.classification !== "system_reserved"
      || !editor.classification.forceOverrideAllowed
    ) {
      return;
    }
    this.setState({
      ...this.state,
      editor: { ...editor, forceOverrideSystem, error: null }
    });
  }

  async save() {
    if (!this.active || !canSaveHotkeyEditor(this.state) || this.state.snapshot === null) {
      return;
    }
    const session = this.session;
    const editor = this.state.editor!;
    const expectedRevision = this.state.snapshot.revision;
    this.setState({ ...this.state, editor: { ...editor, saving: true, error: null } });

    try {
      const { snapshot, systemHotkeyNotice } = await this.client.update({
        actionId: editor.actionId,
        expectedRevision,
        binding: editor.binding,
        forceOverrideSystem:
          editor.classification?.classification === "system_reserved" &&
          editor.classification.forceOverrideAllowed &&
          editor.forceOverrideSystem
      });
      if (!this.active || session !== this.session) {
        return;
      }
      const currentEditor = this.state.editor;
      const sameEditor =
        currentEditor?.actionId === editor.actionId && currentEditor.binding === editor.binding;
      const confirmationIssue = sameEditor ? confirmSavedHotkey(snapshot, currentEditor) : null;
      const savedCleanly = sameEditor && confirmationIssue === null;
      this.setState({
        ...this.state,
        snapshot,
        editor: sameEditor
          ? confirmationIssue
            ? { ...currentEditor, saving: false, error: confirmationIssue }
            : null
          : currentEditor,
        error: null,
        systemHotkeyNotice:
          savedCleanly && systemHotkeyNotice?.restartRequired
            ? systemHotkeyNotice
            : this.state.systemHotkeyNotice
      });
    } catch (error: unknown) {
      if (!this.active || session !== this.session) {
        return;
      }
      let issue = normalizeHotkeyCommandError(error);
      let confirmedSnapshot = this.state.snapshot;
      if (issue.code === "hotkey_revision_conflict") {
        try {
          confirmedSnapshot = await this.client.getSnapshot();
        } catch (refreshError: unknown) {
          issue = normalizeHotkeyCommandError(refreshError);
        }
        if (!this.active || session !== this.session) {
          return;
        }
      }
      const currentEditor = this.state.editor;
      if (currentEditor?.actionId === editor.actionId && currentEditor.binding === editor.binding) {
        this.setState({
          ...this.state,
          snapshot: confirmedSnapshot,
          editor: { ...currentEditor, saving: false, error: issue }
        });
      } else {
        this.setState({ ...this.state, error: issue });
      }
    }
  }

  dismissSystemHotkeyNotice() {
    if (this.state.systemHotkeyNotice === null) {
      return;
    }
    this.setState({ ...this.state, systemHotkeyNotice: null });
  }

  private classifyBinding(binding: string, actionId: GlobalHotkeyId) {
    const request = ++this.classificationRequest;
    const session = this.session;
    void this.client
      .classify(binding)
      .then((classification) => {
        const editor = this.state.editor;
        if (
          !this.active ||
          session !== this.session ||
          request !== this.classificationRequest ||
          editor?.actionId !== actionId ||
          editor.binding !== binding
        ) {
          return;
        }
        this.setState({
          ...this.state,
          editor: {
            ...editor,
            classificationStatus: "ready",
            classification,
            error: null
          }
        });
      })
      .catch((error: unknown) => {
        const editor = this.state.editor;
        if (
          !this.active ||
          session !== this.session ||
          request !== this.classificationRequest ||
          editor?.actionId !== actionId ||
          editor.binding !== binding
        ) {
          return;
        }
        this.setState({
          ...this.state,
          editor: {
            ...editor,
            classificationStatus: "error",
            classification: null,
            error: normalizeHotkeyCommandError(error)
          }
        });
      });
  }

  private setState(state: HotkeyControllerState) {
    this.state = state;
    for (const listener of this.listeners) {
      listener();
    }
  }
}
