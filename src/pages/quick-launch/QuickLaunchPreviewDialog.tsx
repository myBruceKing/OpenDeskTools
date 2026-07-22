import { useEffect, useState } from "react";
import type { ToolMenuLayout, ToolMenuPreferences } from "../../app/quickLaunchClient";
import type { QuickLaunchApp } from "../../app/quickLaunchModel";
import { PreviewFrame } from "../../components/layout/PreviewFrame";
import { ToolMenuPreview, type ToolMenuPreviewItem } from "../../components/patterns/ToolMenuPreview";
import { Button } from "../../components/primitives/Button";
import { DialogShell } from "../../components/primitives/Dialog";
import { SegmentedControl, Toggle } from "../../components/primitives/SelectionControl";
import styles from "./QuickLaunchPage.module.css";

type QuickLaunchPreviewDialogProps = {
  open: boolean;
  preferences: ToolMenuPreferences;
  items: ToolMenuPreviewItem[];
  visibleApps: QuickLaunchApp[];
  loading: boolean;
  onClose: () => void;
  onApply: (preferences: ToolMenuPreferences) => Promise<void>;
  onReorder: (activePath: string, overPath: string) => Promise<void>;
  onSwap: (activePath: string, overPath: string) => Promise<void>;
};

export function QuickLaunchPreviewDialog({
  open,
  preferences,
  items,
  visibleApps,
  loading,
  onClose,
  onApply,
  onReorder,
  onSwap
}: QuickLaunchPreviewDialogProps) {
  const [layout, setLayout] = useState<ToolMenuLayout>(preferences.layout);
  const [keepOpenOnKeyRelease, setKeepOpenOnKeyRelease] = useState(preferences.keepOpenOnKeyRelease);
  const [applying, setApplying] = useState(false);
  const [reordering, setReordering] = useState(false);

  useEffect(() => {
    if (!open) return;
    setLayout(preferences.layout);
    setKeepOpenOnKeyRelease(preferences.keepOpenOnKeyRelease);
  }, [open, preferences.keepOpenOnKeyRelease, preferences.layout]);

  const apply = async () => {
    setApplying(true);
    try {
      await onApply({ layout, keepOpenOnKeyRelease });
      onClose();
    } finally {
      setApplying(false);
    }
  };

  const reorder = async (activeId: string, overId: string) => {
    if (reordering || loading) return;
    const active = visibleApps.find((app) => (app.id ?? app.path) === activeId);
    const over = visibleApps.find((app) => (app.id ?? app.path) === overId);
    if (!active || !over || active.path === over.path) return;
    setReordering(true);
    try {
      // A radial drop replaces a physical slot. List-style insertion would
      // shift all following icons and can move an outer-ring item inward.
      await (layout === "wheel"
        ? onSwap(active.path, over.path)
        : onReorder(active.path, over.path));
    } finally {
      setReordering(false);
    }
  };

  return (
    <DialogShell
      open={open}
      title="快速预览"
      description="选择实际工具盘的显示样式与按键松开后的行为；可直接拖动图标调整固定程序顺序，点击应用后保存显示设置。"
      onClose={onClose}
      footer={
        <>
          <div className={styles.previewKeepOpen}>
            <Toggle
              checked={keepOpenOnKeyRelease}
              label="松开按键后保持显示"
              disabled={applying}
              onChange={setKeepOpenOnKeyRelease}
            />
            <span>松开按键后保持显示</span>
          </div>
          <div className={styles.previewFooterActions}>
            <Button size="inline" disabled={applying} onClick={() => void apply()}>
              {applying ? "应用中…" : "应用"}
            </Button>
            <Button size="inline" disabled={applying} onClick={onClose}>关闭</Button>
          </div>
        </>
      }
    >
      <div className={styles.previewDialog}>
        <SegmentedControl
          label="快速预览形态"
          value={layout}
          options={[
            { value: "wheel", label: "圆形" },
            { value: "dock", label: "横向" },
            { value: "vertical", label: "纵向" }
          ]}
          onChange={setLayout}
        />
        <PreviewFrame className={styles.previewDialogStage}>
          <ToolMenuPreview
            variant={layout}
            items={items}
            size="settings"
            fit="container"
            onItemReorder={reordering || loading
              ? undefined
              : (active, over) => void reorder(active.id, over.id)}
          />
        </PreviewFrame>
      </div>
    </DialogShell>
  );
}
