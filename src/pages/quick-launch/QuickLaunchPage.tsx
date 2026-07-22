import { useEffect, useMemo, useRef, useState } from "react";
import { quickLaunchClient, type ToolMenuLayout } from "../../app/quickLaunchClient";
import { type QuickLaunchApp, useQuickLaunchViewModel } from "../../app/quickLaunchModel";
import { PageScaffold } from "../../components/layout/PageScaffold";
import { PreviewFrame } from "../../components/layout/PreviewFrame";
import { SettingsCard } from "../../components/layout/SettingsCard";
import { ToolbarRow } from "../../components/layout/ToolbarRow";
import { List, ListRow, ListRowDescription, ListRowTitle } from "../../components/patterns/ListRow";
import { SectionTitle } from "../../components/patterns/Section";
import { AppIcon, ToolMenuPreview } from "../../components/patterns/ToolMenuPreview";
import { TagBadge } from "../../components/primitives/Badge";
import { Button } from "../../components/primitives/Button";
import { DialogShell } from "../../components/primitives/Dialog";
import { SearchField } from "../../components/primitives/Field";
import { SegmentedControl, Toggle } from "../../components/primitives/SelectionControl";
import styles from "../static/SettingsPages.module.css";

function LazyQuickLaunchIcon({ app }: { app: QuickLaunchApp }) {
  const host = useRef<HTMLSpanElement>(null);
  const [iconSrc, setIconSrc] = useState(app.iconSrc ?? null);
  const requestedPath = useRef<string | null>(null);
  const ownedUrl = useRef<string | null>(null);

  useEffect(() => {
    if (ownedUrl.current) {
      URL.revokeObjectURL(ownedUrl.current);
      ownedUrl.current = null;
    }
    setIconSrc(app.iconSrc ?? null);
    requestedPath.current = null;
  }, [app.iconSrc, app.path]);

  useEffect(() => () => {
    if (ownedUrl.current) URL.revokeObjectURL(ownedUrl.current);
  }, []);

  useEffect(() => {
    const node = host.current;
    if (!node || iconSrc || !app.iconAvailable || requestedPath.current === app.path) return;
    let active = true;
    const load = async () => {
      requestedPath.current = app.path;
      try {
        const icon = await quickLaunchClient.getIcon(app.path);
        const url = URL.createObjectURL(icon);
        if (!active) {
          URL.revokeObjectURL(url);
          return;
        }
        ownedUrl.current = url;
        setIconSrc(url);
      } catch {
        // Keep the shared generic application fallback when Windows cannot
        // provide an icon for this specific program.
      }
    };
    if (typeof IntersectionObserver === "undefined") {
      void load();
      return () => { active = false; };
    }
    const observer = new IntersectionObserver((entries) => {
      if (entries.some((entry) => entry.isIntersecting)) {
        observer.disconnect();
        void load();
      }
    });
    observer.observe(node);
    return () => {
      active = false;
      observer.disconnect();
    };
  }, [app.iconAvailable, app.path, iconSrc]);

  return (
    <span className={styles.lazyAppIcon} ref={host}>
      <AppIcon src={iconSrc} label={`${app.name} 图标`} size="row" />
    </span>
  );
}

export function QuickLaunchPage() {
  const quickLaunch = useQuickLaunchViewModel();
  const [query, setQuery] = useState("");
  const [quickPreviewOpen, setQuickPreviewOpen] = useState(false);
  const [quickPreviewShape, setQuickPreviewShape] = useState<ToolMenuLayout>("wheel");
  const [quickPreviewKeepOpen, setQuickPreviewKeepOpen] = useState(false);
  const [applyingPreview, setApplyingPreview] = useState(false);
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

  const openQuickPreview = () => {
    setQuickPreviewShape(quickLaunch.toolMenu.layout);
    setQuickPreviewKeepOpen(quickLaunch.toolMenu.keepOpenOnKeyRelease);
    setQuickPreviewOpen(true);
  };

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

  const applyQuickPreview = async () => {
    setApplyingPreview(true);
    try {
      await quickLaunch.actions.updateToolMenu({
        layout: quickPreviewShape,
        keepOpenOnKeyRelease: quickPreviewKeepOpen
      });
      setQuickPreviewOpen(false);
    } finally {
      setApplyingPreview(false);
    }
  };

  return (
    <PageScaffold title="悬浮与快速启动" description="管理悬浮菜单中的固定程序、显示顺序和预览形态。">
      <div className={styles.quickGrid}>
        <SettingsCard fill>
          <div className={styles.panelHeader}>
            <div className={styles.panelTitleGroup}>
              <SectionTitle>已固定</SectionTitle>
              <TagBadge tone="blue">{String(quickLaunch.visiblePinnedApps.length)}</TagBadge>
            </div>
            <Button
              size="inline"
              disabled={quickLaunch.previewItems.length === 0 || quickLaunch.loading}
              onClick={openQuickPreview}
            >
              快速预览
            </Button>
          </div>
          <List className={styles.pinnedList}>
            {quickLaunch.pinnedApps.length === 0 ? (
              <div className={styles.emptyState}>{quickLaunch.loading ? "正在读取已固定程序…" : "尚无已固定程序，可从右侧搜索结果添加。"}</div>
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
                    className={styles.dragDots}
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
                  <LazyQuickLaunchIcon app={app} />
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
                  <div className={styles.quickLaunchActions}>
                    <Button size="inline" disabled={!app.available || !canManageApps} onClick={() => void quickLaunch.actions.launchApp(app.path)}>启动</Button>
                    <Button size="inline" disabled={!canManageApps} onClick={() => void quickLaunch.actions.removePinnedApp(app.path)}>移除</Button>
                  </div>
                </ListRow>
              ))
            )}
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
              disabled={quickLaunch.loading}
              onChange={(event) => setQuery(event.target.value)}
            />
            <Button disabled={quickLaunch.loading} onClick={() => void quickLaunch.actions.refresh()}>重新扫描</Button>
            <Button disabled={quickLaunch.loading} onClick={() => void quickLaunch.actions.addManually()}>手动添加</Button>
          </ToolbarRow>
          <List className={styles.discoveredList}>
            {filteredDiscoveredApps.length === 0 ? (
              <div className={styles.emptyState}>{quickLaunch.loading ? "正在扫描桌面和开始菜单…" : "尚未发现可添加程序，也可以手动添加。"}</div>
            ) : filteredDiscoveredApps.map((app) => {
              const isPinned = quickLaunch.pinnedApps.some((pinnedApp) => pinnedApp.path === app.path);

              return (
                <ListRow className={styles.discoveredRow} key={app.id ?? app.path}>
                  <span className={styles.checkBox} aria-hidden="true" />
                  <LazyQuickLaunchIcon app={app} />
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
      {quickLaunch.error ? <div className={styles.quickLaunchStatus} role="alert">{quickLaunch.error}</div> : null}
      <DialogShell
        open={quickPreviewOpen}
        title="快速预览"
        description="选择实际工具盘的显示样式与按键松开后的行为；点击应用后立即生效。"
        onClose={() => setQuickPreviewOpen(false)}
        footer={
          <>
            <div className={styles.quickPreviewKeepOpen}>
              <Toggle
                checked={quickPreviewKeepOpen}
                label="松开按键后保持显示"
                disabled={applyingPreview}
                onChange={setQuickPreviewKeepOpen}
              />
              <span>松开按键后保持显示</span>
            </div>
            <div className={styles.quickPreviewFooterActions}>
              <Button size="inline" disabled={applyingPreview} onClick={() => void applyQuickPreview()}>
                {applyingPreview ? "应用中…" : "应用"}
              </Button>
              <Button size="inline" disabled={applyingPreview} onClick={() => setQuickPreviewOpen(false)}>关闭</Button>
            </div>
          </>
        }
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
    </PageScaffold>
  );
}
