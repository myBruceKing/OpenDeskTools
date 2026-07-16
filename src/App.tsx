import {
  Clipboard,
  Crop,
  Gauge,
  Home,
  Keyboard,
  Palette,
  Rocket,
  Settings,
  ShieldCheck
} from "lucide-react";
import { useMemo, useState } from "react";

type PageId =
  | "overview"
  | "shortcuts"
  | "launcher"
  | "clipboard"
  | "capture"
  | "theme"
  | "general";

type NavItem = {
  id: PageId;
  label: string;
  icon: React.ComponentType<{ size?: number }>;
};

const navItems: NavItem[] = [
  { id: "overview", label: "概览", icon: Home },
  { id: "shortcuts", label: "快捷键", icon: Keyboard },
  { id: "launcher", label: "快速启动", icon: Rocket },
  { id: "clipboard", label: "剪贴板", icon: Clipboard },
  { id: "capture", label: "截图与二维码", icon: Crop },
  { id: "theme", label: "悬浮与主题", icon: Palette },
  { id: "general", label: "常规", icon: Settings }
];

const pageDescriptions: Record<PageId, string> = {
  overview: "查看后台服务状态、常用快捷键和工具盘预览。",
  shortcuts: "管理 F1 截图、F4 剪贴板二维码和工具盘快捷键。",
  launcher: "选择、排序和修复快速启动里的常用程序。",
  clipboard: "管理剪贴板历史、收藏、保留策略和敏感内容规则。",
  capture: "配置截图、保存、贴图和二维码识别/生成工作流。",
  theme: "统一悬浮窗、临时面板和设置窗口的视觉风格。",
  general: "管理开机启动、数据位置、更新和诊断选项。"
};

const shortcutRows = [
  { keyName: "F1", title: "区域截图", state: "正常" },
  { keyName: "F4", title: "剪贴板二维码", state: "正常" },
  { keyName: "Alt+Space", title: "工具盘", state: "正常" },
  { keyName: "Ctrl+Shift+V", title: "剪贴板面板", state: "待配置" }
];

function App() {
  const [activePage, setActivePage] = useState<PageId>("overview");
  const activeNav = useMemo(
    () => navItems.find((item) => item.id === activePage) ?? navItems[0],
    [activePage]
  );

  return (
    <main className="app-shell">
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-mark">OD</div>
          <div>
            <div className="brand-name">OpenDeskTools</div>
            <div className="brand-subtitle">后台桌面工具箱</div>
          </div>
        </div>

        <nav className="nav-list" aria-label="主导航">
          {navItems.map((item) => {
            const Icon = item.icon;
            const isActive = item.id === activePage;
            return (
              <button
                className={`nav-item${isActive ? " nav-item-active" : ""}`}
                key={item.id}
                onClick={() => setActivePage(item.id)}
                type="button"
              >
                <Icon size={18} />
                <span>{item.label}</span>
              </button>
            );
          })}
        </nav>

        <div className="sidebar-footer">
          <div className="status-pill">
            <span className="status-dot" />
            后台服务运行中
          </div>
        </div>
      </aside>

      <section className="content">
        <header className="content-header">
          <div>
            <p className="eyebrow">设置中心</p>
            <h1>{activeNav.label}</h1>
            <p className="page-description">{pageDescriptions[activePage]}</p>
          </div>
          <button className="primary-action" type="button">
            保存设置
          </button>
        </header>

        <section className="dashboard-grid">
          <article className="panel span-two">
            <div className="panel-heading">
              <div>
                <h2>核心快捷键</h2>
                <p>高频动作保持独立入口，工具盘用于发现和补充。</p>
              </div>
              <ShieldCheck size={22} />
            </div>
            <div className="shortcut-list">
              {shortcutRows.map((row) => (
                <div className="shortcut-row" key={row.keyName}>
                  <kbd>{row.keyName}</kbd>
                  <div>
                    <strong>{row.title}</strong>
                    <span>{row.state}</span>
                  </div>
                  <button type="button">编辑</button>
                </div>
              ))}
            </div>
          </article>

          <article className="panel overlay-preview">
            <div className="panel-heading">
              <div>
                <h2>临时面板预览</h2>
                <p>悬浮与快捷键呼出的界面使用 graphite 风格。</p>
              </div>
            </div>
            <div className="radial-preview">
              <div className="radial-center">×</div>
              <span>截图</span>
              <span>剪贴板</span>
              <span>二维码</span>
              <span>启动</span>
            </div>
          </article>

          <article className="panel">
            <div className="panel-heading">
              <div>
                <h2>当前页面</h2>
                <p>{pageDescriptions[activePage]}</p>
              </div>
              <Gauge size={22} />
            </div>
            <div className="page-card">
              <strong>{activeNav.label}</strong>
              <span>页面骨架已就绪，后续接入真实能力。</span>
            </div>
          </article>

          <article className="panel span-two">
            <div className="panel-heading">
              <div>
                <h2>第一阶段目标</h2>
                <p>先固定桌面壳、主题、导航和后台工作方式。</p>
              </div>
            </div>
            <div className="milestone-list">
              <span>Tray</span>
              <span>Global Shortcuts</span>
              <span>Non-Activating Surfaces</span>
              <span>Quick Launcher</span>
              <span>Clipboard</span>
              <span>Screenshot</span>
              <span>QR</span>
            </div>
          </article>
        </section>
      </section>
    </main>
  );
}

export default App;
