import { useMemo, useState } from "react";
import { useQuickLaunchViewModel, type QuickLaunchApp } from "../../app/quickLaunchModel";
import { PageScaffold } from "../../components/layout/PageScaffold";
import { PreviewFrame } from "../../components/layout/PreviewFrame";
import { SettingsCard } from "../../components/layout/SettingsCard";
import { ToolbarRow } from "../../components/layout/ToolbarRow";
import { List, ListRow, ListRowDescription, ListRowTitle } from "../../components/patterns/ListRow";
import { SectionTitle } from "../../components/patterns/Section";
import { AppIcon, ToolMenuPreview } from "../../components/patterns/ToolMenuPreview";
import { TagBadge } from "../../components/primitives/Badge";
import { Button } from "../../components/primitives/Button";
import { DialogShell, InputDialog } from "../../components/primitives/Dialog";
import { SearchField } from "../../components/primitives/Field";
import { SegmentedControl, Toggle } from "../../components/primitives/SelectionControl";
import styles from "../static/SettingsPages.module.css";

export function QuickLaunchPage() {
  const quickLaunch = useQuickLaunchViewModel();
  const [query, setQuery] = useState("");
  const [quickPreviewOpen, setQuickPreviewOpen] = useState(false);
  const [quickPreviewShape, setQuickPreviewShape] = useState<"wheel" | "dock" | "vertical">("wheel");
  const [draggingAppName, setDraggingAppName] = useState<string | null>(null);
  const [dragOverAppName, setDragOverAppName] = useState<string | null>(null);
  const [manualDialogOpen, setManualDialogOpen] = useState(false);
  const [manualApp, setManualApp] = useState({ name: "", path: "" });
  const filteredDiscoveredApps = useMemo(() => {
    const keyword = query.trim().toLocaleLowerCase();

    if (!keyword) {
      return quickLaunch.discoveredApps;
    }

    return quickLaunch.discoveredApps.filter((app) =>
      `${app.name} ${app.source} ${app.path}`.toLocaleLowerCase().includes(keyword)
    );
  }, [query, quickLaunch.discoveredApps]);

  return (
    <PageScaffold title="悬浮与快速启动" description="管理悬浮菜单中的固定程序、显示顺序和预览形态。">
      <div className={styles.quickGrid}>
        <SettingsCard fill>
          <div className={styles.panelHeader}>
            <div className={styles.panelTitleGroup}>
              <SectionTitle>已固定</SectionTitle>
              <TagBadge tone="blue">{String(quickLaunch.visiblePinnedApps.length)}</TagBadge>
            </div>
            <Button size="inline" onClick={() => setQuickPreviewOpen(true)}>快速预览</Button>
          </div>
          <List className={styles.pinnedList}>
            {quickLaunch.pinnedApps.map((app) => (
              <ListRow
                className={[
                  styles.appRow,
                  draggingAppName === app.name ? styles.appRowDragging : "",
                  dragOverAppName === app.name && draggingAppName !== app.name ? styles.appRowDropTarget : ""
                ]
                  .filter(Boolean)
                  .join(" ")}
                key={app.name}
                draggable
                onDragStart={(event) => {
                  setDraggingAppName(app.name);
                  event.dataTransfer.effectAllowed = "move";
                  event.dataTransfer.setData("text/plain", app.name);
                }}
                onDragEnter={() => {
                  if (draggingAppName && draggingAppName !== app.name) {
                    setDragOverAppName(app.name);
                  }
                }}
                onDragOver={(event) => {
                  event.preventDefault();
                  event.dataTransfer.dropEffect = "move";
                }}
                onDragEnd={() => {
                  setDraggingAppName(null);
                  setDragOverAppName(null);
                }}
                onDrop={(event) => {
                  event.preventDefault();
                  const activeName = event.dataTransfer.getData("text/plain") || draggingAppName;
                  if (activeName) {
                    quickLaunch.actions.reorderPinnedApp(activeName, app.name);
                  }
                  setDraggingAppName(null);
                  setDragOverAppName(null);
                }}
              >
                <span className={styles.dragDots} aria-hidden="true">⋮⋮</span>
                <AppIcon src={app.iconSrc} label={`${app.name} 图标`} size="row" />
                <div className={styles.rowMain}>
                  <ListRowTitle>{app.name}</ListRowTitle>
                  <ListRowDescription>{app.path}</ListRowDescription>
                </div>
                <Toggle
                  checked={quickLaunch.visibleAppNames.has(app.name)}
                  label={`${app.name}显示在工具盘`}
                  onChange={(checked) => quickLaunch.actions.setAppVisible(app.name, checked)}
                />
              </ListRow>
            ))}
          </List>
        </SettingsCard>
        <SettingsCard fill>
          <div className={styles.panelHeader}>
            <SectionTitle>搜索与发现程序</SectionTitle>
          </div>
          <ToolbarRow className={styles.discoveredToolbar} layout="grid">
            <SearchField
              className={styles.discoveredSearch}
              placeholder="搜索已发现的程序"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
            />
            <Button disabled>重新扫描</Button>
            <Button onClick={() => setManualDialogOpen(true)}>手动添加</Button>
          </ToolbarRow>
          <List className={styles.discoveredList}>
            {filteredDiscoveredApps.map((app) => {
              const isPinned = quickLaunch.pinnedApps.some((pinnedApp) => pinnedApp.name === app.name);

              return (
                <ListRow className={styles.discoveredRow} key={app.name}>
                  <span className={styles.checkBox} aria-hidden="true" />
                  <AppIcon src={app.iconSrc} label={`${app.name} 图标`} size="row" />
                  <div className={styles.rowMain}>
                    <ListRowTitle>{app.name}</ListRowTitle>
                    <ListRowDescription>{app.source} · {app.path}</ListRowDescription>
                  </div>
                  <Button size="inline" disabled={isPinned} onClick={() => quickLaunch.actions.addPinnedApp(app)}>
                    {isPinned ? "已添加" : "添加"}
                  </Button>
                </ListRow>
              );
            })}
          </List>
        </SettingsCard>
      </div>
      <DialogShell
        open={quickPreviewOpen}
        title="快速预览"
        description="预览当前已固定程序在悬浮菜单中的显示形态。"
        onClose={() => setQuickPreviewOpen(false)}
        footer={<Button size="inline" onClick={() => setQuickPreviewOpen(false)}>关闭</Button>}
      >
        <div className={styles.quickPreviewDialog}>
          <SegmentedControl
            label="快速预览形态"
            value={quickPreviewShape}
            options={[
              { value: "wheel", label: "圆形" },
              { value: "dock", label: "横向" },
              { value: "vertical", label: "纵向" }
            ]}
            onChange={setQuickPreviewShape}
          />
          <PreviewFrame className={styles.quickPreviewDialogStage}>
            <ToolMenuPreview
              variant={quickPreviewShape}
              items={quickLaunch.previewItems}
              size="settings"
              fit="container"
            />
          </PreviewFrame>
        </div>
      </DialogShell>
      <InputDialog
        open={manualDialogOpen}
        title="手动添加程序"
        description="当前只加入页面预览；真实路径校验和启动会在快速启动服务接入后生效。"
        fields={[
          { name: "name", label: "程序名称", value: manualApp.name, placeholder: "例如 Everything" },
          { name: "path", label: "程序路径", value: manualApp.path, placeholder: "C:\\Program Files\\Everything\\Everything.exe" }
        ]}
        confirmText="添加到预览"
        onChange={(name, value) => setManualApp((current) => ({ ...current, [name]: value }))}
        onConfirm={() => {
          const nextApp: QuickLaunchApp = { name: manualApp.name.trim(), path: manualApp.path.trim() };
          quickLaunch.actions.addPinnedApp(nextApp);
          setManualApp({ name: "", path: "" });
          setManualDialogOpen(false);
        }}
        onClose={() => setManualDialogOpen(false)}
      />
    </PageScaffold>
  );
}
