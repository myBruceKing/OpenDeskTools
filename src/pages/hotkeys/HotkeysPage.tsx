import {
  GLOBAL_HOTKEY_DEFINITIONS,
  toHotkeyBadgeState,
  type HotkeyClassificationKind,
  type HotkeyControllerState
} from "../../app/hotkeyModel";
import { useEffect, useRef, useState } from "react";
import { hotkeyCaptureClient } from "../../app/hotkeyCaptureClient";
import { canSaveHotkeyEditor } from "../../app/hotkeyController";
import { useHotkeyCaptureSession } from "../../app/useHotkeyCaptureSession";
import { useHotkeyController } from "../../app/useHotkeyController";
import { PageScaffold } from "../../components/layout/PageScaffold";
import { SettingsCard } from "../../components/layout/SettingsCard";
import { HotkeyList, type HotkeyListItem } from "../../components/patterns/HotkeyList";
import { ListRowDescription, ListRowTitle } from "../../components/patterns/ListRow";
import { SectionTitle } from "../../components/patterns/Section";
import { Button } from "../../components/primitives/Button";
import { DialogShell, NoticeDialog } from "../../components/primitives/Dialog";
import {
  ShortcutCaptureField,
  type ShortcutCaptureFieldHandle
} from "../../components/primitives/Field";
import { HintTooltip } from "../../components/primitives/HintTooltip";
import { InlineNotice, type InlineNoticeVariant } from "../../components/primitives/InlineNotice";
import { SwitchRow } from "../static/SettingsRows";
import styles from "../static/SettingsPages.module.css";

function createListItems(state: HotkeyControllerState): HotkeyListItem[] {
  if (state.snapshot === null) {
    return [];
  }
  const actions = new Map(state.snapshot.actions.map((action) => [action.actionId, action]));
  return GLOBAL_HOTKEY_DEFINITIONS.flatMap((definition) => {
    const action = actions.get(definition.id);
    if (!action) {
      return [];
    }
    return [{
      id: definition.id,
      title: definition.title,
      description: definition.description,
      binding: action.binding,
      enabled: action.configuredEnabled,
      state: action.actionAvailable ? toHotkeyBadgeState(action.runtimeState) : "unavailable",
      detail: action.detail
    }];
  });
}

const classificationFallback: Record<HotkeyClassificationKind, string> = {
  ordinary: "当前组合可以保存。",
  system_reserved: "该组合属于系统保留快捷键，强制覆盖可能改变系统原有行为。",
  blocked: "该组合被系统禁止注册，不能保存。",
  unsupported_sequence: "当前连续按键序列不支持注册为全局快捷键，不能保存。"
};

const WIN_SINGLE_LETTER_BINDING = /^Win\+[A-Za-z0-9]$/;

function isWinSingleLetterBinding(normalizedBinding: string) {
  return WIN_SINGLE_LETTER_BINDING.test(normalizedBinding);
}

function classificationVariant(classification: HotkeyClassificationKind): InlineNoticeVariant {
  if (classification === "ordinary") {
    return "success";
  }
  if (classification === "system_reserved") {
    return "warning";
  }
  return "error";
}

export function HotkeysPage({ onSnapshotChanged }: { onSnapshotChanged: () => Promise<void> }) {
  const hotkeys = useHotkeyController();
  const captureFieldRef = useRef<ShortcutCaptureFieldHandle>(null);
  const editorActionRef = useRef<Promise<void> | null>(null);
  const editorActionMountedRef = useRef(true);
  const [editorActionPending, setEditorActionPending] = useState(false);
  const nativeCapture = useHotkeyCaptureSession({
    client: hotkeyCaptureClient,
    onToken: (token) => captureFieldRef.current?.acceptNativeToken(token)
  });
  const { state } = hotkeys;
  const editor = state.editor;
  const actionDefinition = editor
    ? GLOBAL_HOTKEY_DEFINITIONS.find((definition) => definition.id === editor.actionId)
    : null;
  const listItems = createListItems(state);
  const classification = editor?.classification;
  const description =
    state.status === "loading"
      ? "正在读取快捷键配置…"
      : state.status === "unavailable"
        ? "快捷键服务当前不可用，配置与注册状态暂不可读取。"
        : "编辑时会按 Windows 快捷键规则实时分类；实际注册时仍可能与其他程序冲突。";

  useEffect(() => {
    if (state.status === "ready" && state.snapshot !== null) {
      void onSnapshotChanged();
    }
  }, [onSnapshotChanged, state.snapshot?.revision, state.status]);

  useEffect(() => {
    editorActionMountedRef.current = true;
    return () => {
      editorActionMountedRef.current = false;
    };
  }, []);

  useEffect(() => {
    if (editor === null && nativeCapture.status !== "idle") {
      void nativeCapture.stop();
    }
  }, [editor, nativeCapture.status, nativeCapture.stop]);

  const runAfterCaptureStopped = (action: () => void | Promise<void>) => {
    if (editorActionRef.current !== null) {
      return;
    }
    setEditorActionPending(true);
    let actionPromise!: Promise<void>;
    actionPromise = (async () => {
      if (await nativeCapture.stop()) {
        await action();
      }
    })().finally(() => {
      if (editorActionRef.current === actionPromise) {
        editorActionRef.current = null;
      }
      if (editorActionMountedRef.current) {
        setEditorActionPending(false);
      }
    });
    editorActionRef.current = actionPromise;
  };

  const closeEditor = () => {
    runAfterCaptureStopped(hotkeys.closeEditor);
  };

  const saveEditor = () => {
    runAfterCaptureStopped(hotkeys.save);
  };

  return (
    <PageScaffold title="快捷键" description={description}>
      {state.error && <InlineNotice variant="error">{state.error.message}</InlineNotice>}
      <div className={styles.hotkeyLayout}>
        <SettingsCard fill>
          <div className={styles.panelHeader}>
            <SectionTitle>全局快捷键</SectionTitle>
            <HintTooltip symbol="i" content="配置、运行状态和冲突结论均来自快捷键服务。" />
          </div>
          {state.status === "ready" && listItems.length > 0 ? (
            <HotkeyList
              hotkeys={listItems}
              density="full"
              toggleDisabled={state.pendingEnabledActionId !== null}
              editDisabled={state.pendingEnabledActionId !== null}
              onEnabledChange={hotkeys.setEnabled}
              onEdit={hotkeys.openEditor}
            />
          ) : (
            <div className={styles.emptyState}>
              {state.status === "loading" ? "正在加载快捷键…" : "快捷键配置当前不可用。"}
            </div>
          )}
        </SettingsCard>
        <SettingsCard fill>
          <SectionTitle>编辑规则</SectionTitle>
          <div className={styles.optionStack}>
            <div>
              <ListRowTitle>普通组合</ListRowTitle>
              <ListRowDescription>规则未命中系统组合时可以直接保存，实际注册仍会检查冲突。</ListRowDescription>
            </div>
            <div>
              <ListRowTitle>系统保留组合</ListRowTitle>
              <ListRowDescription>仅在用户明确开启强制覆盖后允许保存。</ListRowDescription>
            </div>
            <div>
              <ListRowTitle>禁止或不支持</ListRowTitle>
              <ListRowDescription>系统禁止组合和连续按键序列不会写入配置。</ListRowDescription>
            </div>
          </div>
        </SettingsCard>
      </div>

      <DialogShell
        open={editor !== null}
        title={actionDefinition ? `编辑${actionDefinition.title}快捷键` : "编辑快捷键"}
        description="第一次录入会替换当前绑定；保存以后端返回的配置和注册状态为准，未持久化或未生效时会保留弹窗并显示原因。"
        onClose={closeEditor}
        footer={
          <>
            <Button
              size="inline"
              disabled={editor?.saving || editorActionPending}
              onClick={closeEditor}
            >
              取消
            </Button>
            <Button
              size="inline"
              variant="primary"
              disabled={!canSaveHotkeyEditor(state) || editorActionPending}
              onClick={saveEditor}
            >
              {editor?.saving ? "保存中…" : "保存"}
            </Button>
          </>
        }
      >
        {editor && (
          <div className={styles.hotkeyEditor}>
            <ShortcutCaptureField
              ref={captureFieldRef}
              value={editor.binding}
              label={`${actionDefinition?.title ?? "当前功能"}快捷键`}
              onChange={hotkeys.setBinding}
              onAppendToken={hotkeys.appendBindingToken}
              onCaptureStart={nativeCapture.start}
              onCaptureStop={nativeCapture.stop}
              autoFocus
            />

            {nativeCapture.status === "starting" && (
              <InlineNotice variant="pending">正在准备系统组合捕获…</InlineNotice>
            )}
            {nativeCapture.status === "stopping" && (
              <InlineNotice variant="pending">正在停止系统组合捕获…</InlineNotice>
            )}
            {nativeCapture.message && (
              <InlineNotice variant="warning">{nativeCapture.message}</InlineNotice>
            )}

            {editor.classificationStatus === "loading" && (
              <InlineNotice variant="pending">正在检测快捷键组合…</InlineNotice>
            )}
            {classification && (
              <InlineNotice
                variant={classificationVariant(classification.classification)}
                role={classification.classification === "ordinary" ? "status" : "alert"}
              >
                {classification.message || classificationFallback[classification.classification]}
              </InlineNotice>
            )}
            {editor.classificationStatus === "error" && editor.error && (
              <InlineNotice variant="error">{editor.error.message}</InlineNotice>
            )}

            {classification?.classification === "system_reserved" && classification.forceOverrideAllowed && (
              <SwitchRow
                title="强制覆盖系统热键"
                description="确认后允许 OpenDeskTools 接管该系统组合；只有保存后状态显示“已注册”才算生效。"
                checked={editor.forceOverrideSystem}
                disabled={editor.saving}
                onChange={hotkeys.setForceOverrideSystem}
              />
            )}

            {classification?.classification === "system_reserved" &&
              editor.forceOverrideSystem &&
              isWinSingleLetterBinding(classification.normalizedBinding) && (
                <InlineNotice variant="warning" role="note">
                  保存后会在系统层禁用该 Win 组合（写入 DisabledHotkeys 注册表），需重启资源管理器或重启电脑才能完全生效；解绑此快捷键或退出 OpenDeskTools 时会自动移除，不会影响你手动禁用的其它组合。
                </InlineNotice>
              )}

            {!editor.actionAvailable && (
              <InlineNotice variant="info">
                当前功能尚未接入；可以保存配置，功能接入后生效。当前状态不会显示为已注册。
              </InlineNotice>
            )}
            {editor.error && editor.classificationStatus !== "error" && (
              <InlineNotice variant="error">{editor.error.message}</InlineNotice>
            )}
          </div>
        )}
      </DialogShell>

      <NoticeDialog
        open={state.systemHotkeyNotice !== null}
        title="需要重启资源管理器"
        description={
          state.systemHotkeyNotice
            ? `已在系统层禁用 ${state.systemHotkeyNotice.binding}（写入 DisabledHotkeys 注册表）。请重启资源管理器或重启电脑后，OpenDeskTools 的接管才会完全生效；解绑此快捷键或退出 OpenDeskTools 时会自动移除。`
            : undefined
        }
        onClose={hotkeys.dismissSystemHotkeyNotice}
      />
    </PageScaffold>
  );
}
