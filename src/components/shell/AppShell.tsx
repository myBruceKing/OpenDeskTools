import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  Color24Regular,
  Home24Regular,
  Keyboard24Regular,
  QuestionCircle20Regular,
  Rocket24Regular,
  Screenshot24Regular,
  Settings20Regular,
  Settings24Regular
} from "@fluentui/react-icons";
import type { CSSProperties, MouseEvent, ReactNode } from "react";
import type { ServiceState } from "../../app/overviewModel";
import type { ThemeRootPresentation } from "../../app/themeRuntime";
import brandMarkDarkUrl from "../../assets/opendesktools-mark-on-dark.svg";
import brandMarkLightUrl from "../../assets/opendesktools-mark.svg";
import { ClipboardWithLinesIcon } from "../icons/ClipboardWithLinesIcon";
import styles from "./AppShell.module.css";

export type AppRoute = "overview" | "hotkeys" | "quickLaunch" | "clipboard" | "captureQr" | "floatingTheme" | "general";

type AppShellProps = {
  serviceState: ServiceState;
  activeRoute: AppRoute;
  onNavigate: (route: AppRoute) => void;
  theme: ThemeRootPresentation;
  version: string | null;
  footerVariant?: "overview" | "clipboard";
  children: ReactNode;
};

const navItems = [
  { id: "overview", label: "概览", icon: Home24Regular, enabled: true },
  { id: "hotkeys", label: "快捷键", icon: Keyboard24Regular, enabled: true },
  { id: "quickLaunch", label: "悬浮与快速启动", icon: Rocket24Regular, enabled: true },
  { id: "clipboard", label: "剪贴板", icon: ClipboardWithLinesIcon, enabled: true },
  { id: "captureQr", label: "截图与二维码", icon: Screenshot24Regular, enabled: true },
  { id: "floatingTheme", label: "主题", icon: Color24Regular, enabled: true },
  { id: "general", label: "常规", icon: Settings24Regular, enabled: true }
] as const;

const serviceCopy: Record<ServiceState, string> = {
  running: "后台服务运行中",
  starting: "后台服务启动中",
  stopped: "后台服务已停止",
  error: "后台服务异常",
  unknown: "后台服务状态未知"
};

function runWindowAction(action: "minimize" | "toggleMaximize" | "close") {
  const appWindow = getCurrentWindow();
  void appWindow[action]().catch(() => undefined);
}

function startWindowDrag(event: MouseEvent<HTMLElement>) {
  if (event.button !== 0) {
    return;
  }

  const target = event.target;
  if (target instanceof HTMLElement && target.closest("button")) {
    return;
  }

  event.preventDefault();
  void getCurrentWindow().startDragging().catch(() => undefined);
}

function TopBar({ serviceState }: { serviceState: ServiceState }) {
  const serviceClasses = [
    styles.servicePill,
    serviceState === "running" ? styles.servicePillRunning : "",
    serviceState === "error" ? styles.servicePillError : ""
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <header className={styles.topbar} onMouseDown={startWindowDrag}>
      <div className={styles.brand}>
        <span className={styles.brandMark} aria-hidden="true">
          <img className={styles.brandMarkLight} src={brandMarkLightUrl} alt="" draggable={false} />
          <img className={styles.brandMarkDark} src={brandMarkDarkUrl} alt="" draggable={false} />
        </span>
        <span className={styles.brandName}>OpenDeskTools</span>
      </div>
      <span className={styles.brandDivider} aria-hidden="true" />
      <div className={serviceClasses} aria-label={serviceCopy[serviceState]}>
        <span className={styles.serviceDot} aria-hidden="true" />
        {serviceCopy[serviceState]}
      </div>
      <div className={styles.windowControls}>
        <button className={styles.windowButton} aria-label="最小化" onClick={() => runWindowAction("minimize")}>
          <span className={[styles.windowGlyph, styles.windowGlyphMinimize].join(" ")} aria-hidden="true" />
        </button>
        <button className={styles.windowButton} aria-label="最大化" onClick={() => runWindowAction("toggleMaximize")}>
          <span className={[styles.windowGlyph, styles.windowGlyphMaximize].join(" ")} aria-hidden="true" />
        </button>
        <button
          className={[styles.windowButton, styles.windowButtonClose].join(" ")}
          aria-label="关闭"
          onClick={() => runWindowAction("close")}
        >
          <span className={[styles.windowGlyph, styles.windowGlyphClose].join(" ")} aria-hidden="true" />
        </button>
      </div>
    </header>
  );
}

function Sidebar({
  activeRoute,
  onNavigate,
  version,
  footerVariant = "overview"
}: Pick<AppShellProps, "activeRoute" | "onNavigate" | "version" | "footerVariant">) {
  return (
    <aside className={styles.sidebar}>
      <nav className={styles.primaryNav} aria-label="主导航">
        {navItems.map((item) => {
          const Icon = item.icon;
          const isActive = item.id === activeRoute;
          const className = [styles.navItem, isActive ? styles.navItemActive : ""]
            .filter(Boolean)
            .join(" ");

          return (
            <button
              className={className}
              type="button"
              key={item.label}
              title={item.label}
              aria-current={isActive ? "page" : undefined}
              disabled={!item.enabled}
              onClick={() => onNavigate(item.id)}
            >
              <Icon aria-hidden="true" />
              <span className={styles.navLabel}>{item.label}</span>
            </button>
          );
        })}
      </nav>
      <div className={styles.sidebarFooter}>
        {footerVariant === "clipboard" ? (
          <>
            <button className={styles.sidebarLink} type="button" title="设置" disabled>
              <Settings20Regular aria-hidden="true" />
              <span className={styles.navLabel}>设置</span>
            </button>
            <span className={styles.sidebarVersion}>{version ?? "—"}</span>
          </>
        ) : (
          <>
            <button className={styles.sidebarLink} type="button" title="帮助与反馈" disabled>
              <QuestionCircle20Regular aria-hidden="true" />
              <span className={styles.navLabel}>帮助与反馈</span>
            </button>
            <button className={styles.sidebarLink} type="button" title="关于" disabled>
              <Settings20Regular aria-hidden="true" />
              <span className={styles.navLabel}>关于</span>
            </button>
          </>
        )}
      </div>
    </aside>
  );
}

export function AppShell({
  serviceState,
  activeRoute,
  onNavigate,
  theme,
  version,
  footerVariant,
  children
}: AppShellProps) {
  const themeStyle = { "--accent-primary": theme.accent } as CSSProperties;

  return (
    <main
      className={styles.shell}
      data-theme={theme.resolvedTheme}
      data-accent={theme.accent}
      data-reduce-transparency={String(theme.reduceTransparency)}
      data-animation-speed={theme.animationSpeed}
      data-reduced-motion={String(theme.reducedMotion)}
      style={themeStyle}
    >
      <TopBar serviceState={serviceState} />
      <Sidebar
        activeRoute={activeRoute}
        onNavigate={onNavigate}
        version={version}
        footerVariant={footerVariant}
      />
      <section className={styles.content} aria-label={navItems.find((item) => item.id === activeRoute)?.label}>
        {children}
      </section>
    </main>
  );
}
