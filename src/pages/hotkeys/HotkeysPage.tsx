import { useState } from "react";
import { GLOBAL_HOTKEY_DEFINITIONS, type GlobalHotkeyId, type HotkeyState } from "../../app/hotkeyModel";
import { PageScaffold } from "../../components/layout/PageScaffold";
import { SettingsCard } from "../../components/layout/SettingsCard";
import { HotkeyList, type HotkeyListItem } from "../../components/patterns/HotkeyList";
import { SectionTitle } from "../../components/patterns/Section";
import { Button } from "../../components/primitives/Button";
import { DialogShell } from "../../components/primitives/Dialog";
import { ShortcutCaptureField } from "../../components/primitives/Field";
import { HintTooltip } from "../../components/primitives/HintTooltip";
import { SwitchRow } from "../static/SettingsRows";
import styles from "../static/SettingsPages.module.css";

type HotkeyPageRow = {
  id: GlobalHotkeyId;
  binding: string;
  title: string;
  description: string;
  state: HotkeyState;
};

function createHotkeyPageRows(): HotkeyPageRow[] {
  return GLOBAL_HOTKEY_DEFINITIONS.map((definition) => ({
    id: definition.id,
    binding: definition.defaultBinding,
    title: definition.title,
    description: definition.description,
    state: definition.id === "clipboardPanel" ? "conflict" : "normal"
  }));
}

export function HotkeysPage() {
  const [rows, setRows] = useState(createHotkeyPageRows);
  const [enabledHotkeys, setEnabledHotkeys] = useState(() =>
    new Map(createHotkeyPageRows().map((item) => [item.id, item.state !== "conflict"]))
  );
  const [editingHotkeyId, setEditingHotkeyId] = useState<GlobalHotkeyId | null>(null);
  const editingRow = rows.find((item) => item.id === editingHotkeyId) ?? null;
  const [shortcutDraft, setShortcutDraft] = useState("");
  const hotkeyListItems: HotkeyListItem[] = rows.map((item) => ({
    ...item,
    enabled: enabledHotkeys.get(item.id) ?? false,
    detail: item.state === "conflict" ? "与系统快捷键冲突" : null
  }));

  return (
    <PageScaffold title="快捷键" description="集中查看快捷键状态。未接入系统注册前，只展示页面结构和冲突状态。">
      <div className={styles.hotkeyLayout}>
        <SettingsCard fill>
          <div className={styles.panelHeader}>
            <SectionTitle>全局快捷键</SectionTitle>
            <HintTooltip symbol="i" content="当前为静态页面。真实注册、修改和冲突检测将在后端快捷键服务完成后接入。" />
          </div>
          <HotkeyList
            hotkeys={hotkeyListItems}
            density="full"
            onEnabledChange={(id, checked) =>
              setEnabledHotkeys((current) => {
                const next = new Map(current);
                next.set(id, checked);
                return next;
              })
            }
            onEdit={(id) => {
              const nextEditingRow = rows.find((item) => item.id === id);
              if (!nextEditingRow) {
                return;
              }

              setEditingHotkeyId(id);
              setShortcutDraft(nextEditingRow.binding);
            }}
            toggleDisabled={(item) => item.state === "conflict"}
            editDisabled={(item) => item.state === "conflict"}
          />
        </SettingsCard>
        <SettingsCard fill>
          <SectionTitle>冲突处理</SectionTitle>
          <div className={styles.optionStack}>
            <SwitchRow title="自动避让" description="检测冲突时切换到备用快捷键" checked />
            <SwitchRow title="提示我解决" description="冲突发生时在设置页展示提示" checked={false} />
            <SwitchRow title="保持当前设置" description="不自动修改用户已设快捷键" checked={false} />
          </div>
        </SettingsCard>
      </div>
      <DialogShell
        open={editingRow !== null}
        title="编辑快捷键"
        description="点击捕获框后直接按键；连续按键会追加到后面。当前只更新页面预览。"
        onClose={() => setEditingHotkeyId(null)}
        footer={
          <>
            <Button size="inline" onClick={() => setShortcutDraft("")}>
              清空
            </Button>
            <Button size="inline" onClick={() => setEditingHotkeyId(null)}>
              取消
            </Button>
            <Button
              size="inline"
              onClick={() => {
                if (!editingRow || !shortcutDraft.trim()) {
                  return;
                }

                setRows((current) =>
                  current.map((item) =>
                    item.id === editingRow.id ? { ...item, binding: shortcutDraft.trim() } : item
                  )
                );
                setEditingHotkeyId(null);
              }}
            >
              应用预览
            </Button>
          </>
        }
      >
        <div className={styles.shortcutEditorRow}>
          <span className={styles.shortcutEditorLabel}>{editingRow ? `${editingRow.title} 快捷键：` : "快捷键："}</span>
          <ShortcutCaptureField
            value={shortcutDraft}
            label={editingRow ? `${editingRow.title} 快捷键捕获框` : "快捷键捕获框"}
            onChange={setShortcutDraft}
          />
        </div>
      </DialogShell>
    </PageScaffold>
  );
}
