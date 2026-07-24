<p align="right">
  <strong>简体中文</strong> · <a href="./README.en.md">English</a>
</p>

# OpenDeskTools

面向 Windows 的本地优先桌面效率工具箱。用统一的全局快捷键、剪贴板历史和快速启动面板，把高频操作留在当前工作流里完成。

> OpenDeskTools 仍处于早期开发阶段，暂未提供正式安装包。当前版本适合参与开发、测试和产品体验验证。

## 我们要做什么

日常使用 Windows 时，复制历史、启动程序、区域截图、屏幕贴图和二维码转换往往分散在不同软件中。OpenDeskTools 希望把这些能力收进一个轻量、可组合、键盘优先的桌面工具箱：

- **随叫随到**：通过全局快捷键直接打开临时面板，不打断当前窗口和输入焦点。
- **本地优先**：剪贴板历史、设置和图标缓存保存在本机，核心流程不依赖云服务。
- **统一体验**：主设置窗口负责管理，独立 Surface 负责快速操作；两者复用同一份数据和业务规则。
- **Windows 原生能力**：快捷键、剪贴板、窗口、托盘和后续截图能力由 Rust / Tauri 与 Windows API 承接。
- **长期可维护**：共享 service、组件和设计令牌，避免每个入口各自维护一套逻辑与样式。

## 当前进度

| 模块 | 状态 | 当前能力 |
| --- | --- | --- |
| 剪贴板 | 已实现，持续验收 | 文本、图片和文件历史；搜索筛选、收藏、删除、编辑、来源图标；独立快捷面板；复制与输入 |
| 快速启动 | 已实现，持续打磨 | 扫描与手动添加程序；固定、排序、显示/隐藏、移除；圆形/横向/纵向启动面板；真实程序图标 |
| 全局快捷键 | 已实现，持续兼容性测试 | 可视化改绑、原生按键捕获、冲突状态、Windows 组合键处理和运行时路由 |
| 二维码 | 已实现，持续验收 | F4 全局快捷键读取最新内部文本生成二维码或从图片识别二维码；结果写回内部历史、尝试同步系统剪贴板，并以不抢焦点的右上角提示反馈 |
| 主题与常规设置 | 已实现，持续验收 | 明暗主题、预设及色盘自定义强调色、动画/透明偏好；主窗口本地 PNG/JPEG/WebP 图片皮肤、背景适配/遮罩/模糊和面板透明度；托盘、开机启动、数据目录迁移和本地诊断开关 |
| F1 区域截图 | 规划中 | 自研多显示器截图内核、区域选择、剪贴板与文件输出 |
| F3 屏幕贴图 | 规划中 | 无边框置顶贴图、拖动、缩放和多实例 |

更细的完成条件、实机验收缺口和阶段顺序见 [开发计划](./docs/development-plan.md)、[图片皮肤、截图和贴图计划](./docs/skin-and-capture-development-plan.md)，截图内核与屏幕贴图的技术落地见 [截图与屏幕贴图开发方案](./docs/screenshot-and-pinning-development-plan.md)。

## 产品结构

OpenDeskTools 由两个互补的交互层组成：

1. **主窗口**：管理快捷键、快速启动、剪贴板、二维码、主题和常规设置。
2. **快捷 Surface**：在当前鼠标或输入位置附近打开剪贴板与工具盘，用完后自然淡出，不把用户带离当前任务。

后端能力通过唯一的 service / manager 收口；Tauri command、全局快捷键、托盘和 React 页面只作为入口适配器。完整边界见 [共享能力地图](./docs/architecture/capability-map.md)。

## 技术栈

- Rust + Tauri 2
- React + TypeScript + Vite
- Windows API / Win32
- SQLite 本地持久化
- Vitest + Rust tests + GitHub Actions

项目目前以 Windows 10 / 11 为主要开发和验证环境。Windows 7 兼容构建属于后续独立验证目标，不代表当前版本已经支持。

## 本地开发

### 环境要求

- Windows 10 或 Windows 11
- Node.js 20 或更高版本
- pnpm 11.1.3（仓库 CI 使用该版本）
- Rust stable 与 MSVC 工具链
- Tauri 2 在 Windows 上所需的 WebView2 和 C++ 构建环境

### 启动开发环境

```powershell
corepack enable
corepack prepare pnpm@11.1.3 --activate
pnpm install --frozen-lockfile
pnpm tauri dev
```

### 构建 Debug 包

项目开发验收默认使用 Debug 包：

```powershell
pnpm tauri build --debug
```

构建产物位于 `src-tauri/target/debug/`。当前 Tauri bundle 配置未开启，因此该命令生成可执行程序，不生成正式安装包。

## 质量检查

提交前建议运行：

```powershell
pnpm check:node
pnpm check:source
pnpm typecheck
pnpm test
pnpm build
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --locked --manifest-path src-tauri/Cargo.toml
cargo clippy --all-targets --locked --manifest-path src-tauri/Cargo.toml -- -D warnings
```

CI 在 Windows runner 上执行同类检查。涉及界面的改动还需要使用同一个 Debug 构建完成真实窗口、真实交互和多尺寸截图验收。

## 仓库结构

```text
src/                         React 页面、共享组件、客户端模型与快捷 Surface
src-tauri/src/               Rust commands、service、manager 与 Windows 基础设施
docs/prototypes/             页面视觉原型与测量记录
docs/architecture/           架构边界与前端设计系统
docs/audits/                 实机验收报告与可复现证据
scripts/                     Node 检查、窗口截图与开发辅助脚本
```

## 路线图

近期工作按以下顺序推进：

1. 完成剪贴板与快速启动在深色、高 DPI、不同 Windows 版本和异常焦点场景下的验收。
2. 实现 OpenDeskTools 自有的 F1 区域截图能力。
3. 在共享图片与窗口能力之上实现 F3 屏幕贴图。
4. 完成发布前的安装、升级、兼容性、性能和隐私检查。

产品视觉以 [页面原型](./docs/prototypes/pages/) 为标准，架构与阶段门禁以 [开发计划](./docs/development-plan.md) 为准。

## 参与项目

欢迎通过 Issue 描述使用场景、复现步骤和期望体验，也欢迎提交聚焦且可验证的 Pull Request。涉及 UI 的改动请同时说明受影响入口、状态、窗口尺寸和真实运行验证结果。

## 许可证

本项目采用 [PolyForm Noncommercial License 1.0.0](./LICENSE)。当前许可证允许非商业用途；商业使用不在授权范围内。
