import React from "react";
import ReactDOM from "react-dom/client";
import App from "./app/App";
import { ClipboardPreviewSurfaceRoot } from "./app/ClipboardPreviewSurfaceRoot";
import { ClipboardSurfaceRoot } from "./app/ClipboardSurfaceRoot";
import { ToolMenuSurfaceRoot } from "./app/ToolMenuSurfaceRoot";
import "./styles/tokens.css";
import "./styles/themes.css";
import "./styles/global.css";

const isClipboardSurface = window.location.hash === "#clipboard-surface";
const isClipboardPreviewSurface = window.location.hash === "#clipboard-preview-surface";
const isToolMenuSurface = window.location.hash === "#tool-menu-surface";
if (isToolMenuSurface) document.title = "";
document.documentElement.dataset.windowSurface = isClipboardPreviewSurface
  ? "clipboard-preview"
  : isClipboardSurface ? "clipboard" : isToolMenuSurface ? "tool-menu" : "main";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {isClipboardPreviewSurface
      ? <ClipboardPreviewSurfaceRoot />
      : isClipboardSurface ? <ClipboardSurfaceRoot /> : isToolMenuSurface ? <ToolMenuSurfaceRoot /> : <App />}
  </React.StrictMode>
);
