import {
  GLOBAL_HOTKEY_DEFINITIONS,
  toHotkeyBadgeState,
  type HotkeyClassificationKind,
  type HotkeyControllerState
} from "../../app/hotkeyModel";
import { useEffect } from "react";
import { canSaveHotkeyEditor } from "../../app/hotkeyController";
import { useHotkeyController } from "../../app/useHotkeyController";
import { PageScaffold } from "../../components/layout/PageScaffold";
import { SettingsCard } from "../../components/layout/SettingsCard";
import { HotkeyList, type HotkeyListItem } from "../../components/patterns/HotkeyList";
import { ListRowDescription, ListRowTitle } from "../../components/patterns/ListRow";
import { SectionTitle } from "../../components/patterns/Section";
import { Button } from "../../components/primitives/Button";
import { DialogShell } from "../../components/primitives/Dialog";
import { ShortcutCaptureField } from "../../components/primitives/Field";
import { HintTooltip } from "../../components/primitives/HintTooltip";
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

function classificationClass(classification: HotkeyClassificationKind) {
  if (classification === "ordinary") {
    return styles.hotkeyClassificationSuccess;
  }
  if (classification === "system_reserved") {
    return styles.hotkeyClassificationWarning;
  }
  return styles.hotkeyClassificationError;
}

export function HotkeysPage({ onSnapshotChanged }: { onSnapshotChanged: () => Promise<void> }) {
  const hotkeys = useHotkeyController();
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

  return (
    <PageScaffold title="快捷键" description={description}>
      {state.error && <div className={styles.hotkeyPageError} role="alert">{state.error.message}</div>}
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
              toggleDisabled
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
        description="按下新组合后会立即分类；系统占用和其他程序冲突会在实际注册时继续检查。"
        onClose={hotkeys.closeEditor}
        footer={
          <>
            <Button size="inline" disabled={editor?.saving} onClick={hotkeys.closeEditor}>取消</Button>
            <Button
              size="inline"
              variant="primary"
              disabled={!canSaveHotkeyEditor(state)}
              onClick={() => void hotkeys.save()}
            >
              {editor?.saving ? "保存中…" : "保存"}
            </Button>
          </>
        }
      >
        {editor && (
          <div className={styles.hotkeyEditor}>
            <ShortcutCaptureField
              value={editor.binding}
              label={`${actionDefinition?.title ?? "当前功能"}快捷键`}
              onChange={hotkeys.setBinding}
              autoFocus
            />

            {editor.classificationStatus === "loading" && (
              <div className={styles.hotkeyClassificationPending} role="status">正在检测快捷键组合…</div>
            )}
            {classification && (
              <div
                className={classificationClass(classification.classification)}
                role={classification.classification === "ordinary" ? "status" : "alert"}
              >
                {classification.message || classificationFallback[classification.classification]}
              </div>
            )}
            {editor.classificationStatus === "error" && editor.error && (
              <div className={styles.hotkeyClassificationError} role="alert">{editor.error.message}</div>
            )}

            {classification?.classification === "system_reserved" && classification.forceOverrideAllowed && (
              <SwitchRow
                title="强制覆盖系统热键"
                description="我了解这可能替换 Windows 或其他系统功能的原有快捷键。"
                checked={editor.forceOverrideSystem}
                disabled={editor.saving}
                onChange={hotkeys.setForceOverrideSystem}
              />
            )}

            {!editor.actionAvailable && (
              <div className={styles.hotkeyActionUnavailable} role="note">
                当前功能尚未接入；可以保存配置，功能接入后生效。当前状态不会显示为已注册。
              </div>
            )}
            {editor.error && editor.classificationStatus !== "error" && (
              <div className={styles.hotkeyClassificationError} role="alert">{editor.error.message}</div>
            )}
          </div>
        )}
      </DialogShell>
    </PageScaffold>
  );
}
