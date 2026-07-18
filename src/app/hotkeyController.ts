import type { HotkeyClient } from "./hotkeyClient";
import {
  normalizeHotkeyCommandError,
  type GlobalHotkeyId,
  type HotkeyControllerState,
  type HotkeyEditorState
} from "./hotkeyModel";

const INITIAL_STATE: HotkeyControllerState = {
  status: "loading",
  snapshot: null,
  editor: null,
  error: null
};

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
        this.setState({ status: "ready", snapshot, editor: null, error: null });
      })
      .catch((error: unknown) => {
        if (!this.active || session !== this.session) {
          return;
        }
        this.setState({
          status: "unavailable",
          snapshot: null,
          editor: null,
          error: normalizeHotkeyCommandError(error)
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
        classificationStatus: "loading",
        classification: null,
        forceOverrideSystem: false,
        error: null
      }
    });
    this.classifyBinding(binding, editor.actionId);
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
      const snapshot = await this.client.update({
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
      this.setState({
        ...this.state,
        snapshot,
        editor: sameEditor ? null : currentEditor,
        error: null
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
