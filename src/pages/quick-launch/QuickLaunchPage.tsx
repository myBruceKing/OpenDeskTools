import { useMemo, useRef, useState } from "react";
import { useQuickLaunchViewModel } from "../../app/quickLaunchModel";
import { PageScaffold } from "../../components/layout/PageScaffold";
import { SettingsCard } from "../../components/layout/SettingsCard";
import { ToolbarRow } from "../../components/layout/ToolbarRow";
import { List, ListRow, ListRowDescription, ListRowTitle } from "../../components/patterns/ListRow";
import { SectionTitle } from "../../components/patterns/Section";
import { TagBadge } from "../../components/primitives/Badge";
import { Button } from "../../components/primitives/Button";
import { SearchField } from "../../components/primitives/Field";
import { Toggle } from "../../components/primitives/SelectionControl";
import { QuickLaunchAppIcon } from "./QuickLaunchAppIcon";
import { QuickLaunchPreviewDialog } from "./QuickLaunchPreviewDialog";
import styles from "./QuickLaunchPage.module.css";
import sharedStyles from "../static/SettingsPages.module.css";

export function QuickLaunchPage() {
  const quickLaunch = useQuickLaunchViewModel();
  const [query, setQuery] = useState("");
  const [quickPreviewOpen, setQuickPreviewOpen] = useState(false);
  const [draggingAppName, setDraggingAppName] = useState<string | null>(null);
  const [dragOverAppName, setDragOverAppName] = useState<string | null>(null);
  const dragPointer = useRef<{ pointerId: number; appPath: string } | null>(null);
  const canManageApps = quickLaunch.sourceAvailable && !quickLaunch.loading;
  const filteredDiscoveredApps = useMemo(() => {
    const keyword = query.trim().toLocaleLowerCase();

    if (!keyword) {
      return quickLaunch.discoveredApps;
    }

    return quickLaunch.discoveredApps.filter((app) =>
      `${app.name} ${app.source} ${app.path}`.toLocaleLowerCase().includes(keyword)
    );
  }, [query, quickLaunch.discoveredApps]);

  const finishPointerReorder = (pointerId: number) => {
    const active = dragPointer.current;
    if (!active || active.pointerId !== pointerId) return;
    const targetPath = dragOverAppName;
    dragPointer.current = null;
    setDraggingAppName(null);
    setDragOverAppName(null);
    if (targetPath && targetPath !== active.appPath) {
      void quickLaunch.actions.reorderPinnedApp(active.appPath, targetPath);
    }
  };

  return (
    <PageScaffold title="悬浮与快速启动" description="管理悬浮菜单中的固定程序、显示顺序和预览形态。">
      <div className={styles.grid}>
        <SettingsCard fill>
          <div className={sharedStyles.panelHeader}>
            <div className={styles.panelTitleGroup}>
              <SectionTitle>已固定</SectionTitle>
              <TagBadge tone="blue">{String(quickLaunch.visiblePinnedApps.length)}</TagBadge>
            </div>
            <Button
              size="inline"
              disabled={quickLaunch.previewItems.length === 0 || quickLaunch.loading}
              onClick={() => setQuickPreviewOpen(true)}
            >
              快速预览
            </Button>
          </div>
          <List className={styles.pinnedList}>
            {quickLaunch.pinnedApps.length === 0 ? (
              <div className={sharedStyles.emptyState}>{quickLaunch.loading ? "正在读取已固定程序…" : "尚无已固定程序，可从右侧搜索结果添加。"}</div>
            ) : (
              quickLaunch.pinnedApps.map((app) => (
                <ListRow
                  className={[
                    styles.appRow,
                    draggingAppName === app.path ? styles.appRowDragging : "",
                    dragOverAppName === app.path && draggingAppName !== app.path ? styles.appRowDropTarget : ""
                  ]
                    .filter(Boolean)
                    .join(" ")}
                  key={app.id ?? app.path}
                  data-quick-launch-path={app.path}
                >
                  <span
                    className={styles.dragHandle}
                    title={quickLaunch.pinnedApps.length > 1 ? "拖动排序" : "添加更多固定程序后可排序"}
                    aria-label={quickLaunch.pinnedApps.length > 1 ? "拖动排序" : "当前只有一项，无法排序"}
                    role="button"
                    tabIndex={canManageApps && quickLaunch.pinnedApps.length > 1 ? 0 : -1}
                    aria-disabled={!canManageApps || quickLaunch.pinnedApps.length <= 1}
                    onPointerDown={(event) => {
                      if (!canManageApps || quickLaunch.pinnedApps.length <= 1 || event.button !== 0) return;
                      event.preventDefault();
                      event.currentTarget.setPointerCapture(event.pointerId);
                      dragPointer.current = { pointerId: event.pointerId, appPath: app.path };
                      setDraggingAppName(app.path);
                      setDragOverAppName(null);
                    }}
                    onPointerMove={(event) => {
                      const active = dragPointer.current;
                      if (!active || active.pointerId !== event.pointerId) return;
                      const target = document.elementFromPoint(event.clientX, event.clientY)
                        ?.closest<HTMLElement>("[data-quick-launch-path]");
                      const targetPath = target?.dataset.quickLaunchPath ?? null;
                      setDragOverAppName(targetPath && targetPath !== active.appPath ? targetPath : null);
                    }}
                    onPointerUp={(event) => {
                      if (event.currentTarget.hasPointerCapture(event.pointerId)) {
                        event.currentTarget.releasePointerCapture(event.pointerId);
                      }
                      finishPointerReorder(event.pointerId);
                    }}
                    onPointerCancel={(event) => {
                      if (event.currentTarget.hasPointerCapture(event.pointerId)) {
                        event.currentTarget.releasePointerCapture(event.pointerId);
                      }
                      if (dragPointer.current?.pointerId === event.pointerId) {
                        dragPointer.current = null;
                        setDraggingAppName(null);
                        setDragOverAppName(null);
                      }
                    }}
                  >⋮⋮</span>
                  <QuickLaunchAppIcon app={app} />
                  <div className={styles.rowMain}>
                    <ListRowTitle>{app.name}</ListRowTitle>
                    <ListRowDescription>{app.path}</ListRowDescription>
                  </div>
                  <Toggle
                    checked={app.visible ?? true}
                    label={`${app.name}显示在工具盘`}
                    disabled={!canManageApps}
                    onChange={(checked) => void quickLaunch.actions.setAppVisible(app.path, checked)}
                  />
                  <div className={styles.actions}>
                    <Button size="inline" disabled={!app.available || !canManageApps} onClick={() => void quickLaunch.actions.launchApp(app.path)}>启动</Button>
                    <Button size="inline" disabled={!canManageApps} onClick={() => void quickLaunch.actions.removePinnedApp(app.path)}>移除</Button>
                  </div>
                </ListRow>
              ))
            )}
          </List>
        </SettingsCard>
        <SettingsCard fill>
          <div className={sharedStyles.panelHeader}>
            <SectionTitle>搜索与发现程序</SectionTitle>
          </div>
          <ToolbarRow className={styles.discoveredToolbar} layout="grid">
            <SearchField
              className={styles.discoveredSearch}
              placeholder="搜索已发现的程序"
              value={query}
              disabled={quickLaunch.loading}
              onChange={(event) => setQuery(event.target.value)}
            />
            <Button disabled={quickLaunch.loading} onClick={() => void quickLaunch.actions.refresh()}>重新扫描</Button>
            <Button disabled={quickLaunch.loading} onClick={() => void quickLaunch.actions.addManually()}>手动添加</Button>
          </ToolbarRow>
          <List className={styles.discoveredList}>
            {filteredDiscoveredApps.length === 0 ? (
              <div className={sharedStyles.emptyState}>{quickLaunch.loading ? "正在扫描桌面和开始菜单…" : "尚未发现可添加程序，也可以手动添加。"}</div>
            ) : filteredDiscoveredApps.map((app) => {
              const isPinned = quickLaunch.pinnedApps.some((pinnedApp) => pinnedApp.path === app.path);

              return (
                <ListRow className={styles.discoveredRow} key={app.id ?? app.path}>
                  <span className={styles.checkBox} aria-hidden="true" />
                  <QuickLaunchAppIcon app={app} />
                  <div className={styles.rowMain}>
                    <ListRowTitle>{app.name}</ListRowTitle>
                    <ListRowDescription>{app.source} · {app.path}</ListRowDescription>
                  </div>
                  <Button
                    size="inline"
                    disabled={!canManageApps || isPinned}
                    onClick={() => void quickLaunch.actions.addPinnedApp(app)}
                  >
                    {isPinned ? "已添加" : "添加"}
                  </Button>
                </ListRow>
              );
            })}
          </List>
        </SettingsCard>
      </div>
      {quickLaunch.error ? <div className={styles.status} role="alert">{quickLaunch.error}</div> : null}
      <QuickLaunchPreviewDialog
        open={quickPreviewOpen}
        preferences={quickLaunch.toolMenu}
        items={quickLaunch.previewItems}
        visibleApps={quickLaunch.visiblePinnedApps}
        loading={quickLaunch.loading}
        onClose={() => setQuickPreviewOpen(false)}
        onApply={quickLaunch.actions.updateToolMenu}
        onReorder={quickLaunch.actions.reorderPinnedApp}
        onSwap={quickLaunch.actions.swapPinnedApps}
      />
    </PageScaffold>
  );
}
