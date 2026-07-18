import edgeIcon from "../../src/assets/app-icons/edge.png";
import explorerIcon from "../../src/assets/app-icons/explorer.png";
import powershellIcon from "../../src/assets/app-icons/powershell.png";
import terminalIcon from "../../src/assets/app-icons/terminal.png";
import type { QuickLaunchApp } from "../../src/app/quickLaunchModel";

// Prototype-only application data for a future visual harness. Production
// models must receive real paths and icons from the native discovery service.
export const PINNED_QUICK_LAUNCH_PREVIEW_APPS: QuickLaunchApp[] = [
  { name: "Cursor", path: "C:\\Users\\Public\\Desktop\\Cursor.lnk" },
  { name: "Visual Studio Code", path: "C:\\Users\\Public\\Desktop\\Visual Studio Code.lnk" },
  { name: "终端", path: "C:\\Users\\sample\\Start Menu\\Terminal.lnk", iconSrc: terminalIcon },
  { name: "文件资源管理器", path: "C:\\Windows\\explorer.exe", iconSrc: explorerIcon },
  { name: "Microsoft Edge", path: "C:\\Apps\\Microsoft Edge\\msedge.exe", iconSrc: edgeIcon }
];

export const DISCOVERED_QUICK_LAUNCH_PREVIEW_APPS: QuickLaunchApp[] = [
  { name: "PowerShell", path: "C:\\Windows\\System32\\WindowsPowerShell\\powershell.exe", source: "桌面", iconSrc: powershellIcon },
  { name: "示例应用", path: "C:\\Apps\\Example\\Example.exe", source: "开始菜单" }
];
