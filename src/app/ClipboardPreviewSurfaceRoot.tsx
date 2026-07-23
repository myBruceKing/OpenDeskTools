import { useEffect, useRef, useState } from "react";
import { clipboardClient, type ClipboardPreviewSurfaceChange } from "./clipboardClient";
import { useClipboardSurfaceUnderlayColor } from "./clipboardSurfaceUnderlay";
import {
  ClipboardHistoryPreviewContent,
  clipboardHistoryPreviewLabel
} from "../components/patterns/ClipboardHistoryItem";
import { useClipboardImagePreview } from "../pages/clipboard/useClipboardImagePreview";
import { useClipboardController } from "./useClipboardController";
import { useWindowSurfaceRuntime } from "./useWindowSurfaceRuntime";
import { useWindowSurfaceMetricsTrace } from "./windowSurfaceMetrics";
import styles from "./ClipboardPreviewSurfaceRoot.module.css";

type PreviewSelectionState = {
  recordId: string | null;
  status: "loading" | "ready" | "closed" | "error";
  message: string | null;
};

const INITIAL_PREVIEW_STATE: PreviewSelectionState = {
  recordId: null,
  status: "loading",
  message: null
};

export function ClipboardPreviewSurfaceRoot() {
  const windowRootRef = useRef<HTMLDivElement>(null);
  const clipboard = useClipboardController(false);
  const [preview, setPreview] = useState(INITIAL_PREVIEW_STATE);
  const theme = useWindowSurfaceRuntime();
  useClipboardSurfaceUnderlayColor(theme.resolvedTheme, clipboardClient.setSurfaceUnderlayColor);
  useWindowSurfaceMetricsTrace("clipboard-preview", windowRootRef, clipboardClient.tracePreviewDebug);

  useEffect(() => {
    let disposed = false;
    let observedChange = 0;
    let unlisten: (() => void) | undefined;
    const applyChange = (change: ClipboardPreviewSurfaceChange) => {
      if (disposed) return;
      observedChange += 1;
      setPreview({
        recordId: change.recordId,
        status: change.visible && change.recordId ? "ready" : "closed",
        message: null
      });
    };

    void (async () => {
      try {
        unlisten = await clipboardClient.subscribePreviewSurface(applyChange);
        if (disposed) {
          unlisten();
          return;
        }
        const queryStartedAfterChange = observedChange;
        const current = await clipboardClient.getPreviewSurfaceState();
        if (!disposed && observedChange === queryStartedAfterChange) {
          setPreview({
            recordId: current.recordId,
            status: current.visible && current.recordId ? "ready" : "closed",
            message: null
          });
        }
      } catch {
        if (!disposed) {
          setPreview({
            recordId: null,
            status: "error",
            message: "预览暂时不可用"
          });
        }
      }
    })();

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const item = preview.recordId
    ? clipboard.state.viewModel.items.find((candidate) => candidate.id === preview.recordId) ?? null
    : null;
  const imagePreview = useClipboardImagePreview(
    item?.kind === "image" ? item.id : null,
    clipboard.loadImage
  );
  const loading = preview.status === "loading" || clipboard.state.status === "loading";
  const unavailable = clipboard.state.status === "unavailable";
  const missing = preview.status === "ready" && !loading && !unavailable && !item;
  const statusMessage = preview.message
    ?? (loading
      ? "正在加载预览…"
      : unavailable
        ? clipboard.state.error?.message ?? "预览内容暂时不可用"
        : missing ? "这条记录已不可用" : "暂无预览内容");

  const publishHover = (inside: boolean) => {
    void clipboardClient.publishPreviewHover({ inside, recordId: preview.recordId }).catch(() => undefined);
  };

  return (
    <div ref={windowRootRef} className={styles.windowRoot}>
      <section
        className={styles.previewSurface}
        aria-label={item ? `剪贴板预览：${item.title}` : "剪贴板预览"}
        data-window-border-layer="true"
        onPointerEnter={() => publishHover(true)}
        onPointerLeave={() => publishHover(false)}
      >
        {item ? (
          <>
            <header className={styles.header}>
              <strong>{clipboardHistoryPreviewLabel(item.kind)}</strong>
              <span title={item.sourceApp}>{item.sourceApp}</span>
            </header>
            <ClipboardHistoryPreviewContent
              className={styles.content}
              item={item}
              imagePreview={imagePreview.state}
              onImageLoaded={imagePreview.markLoaded}
              onImageError={imagePreview.markDecodeError}
              onRetryImage={imagePreview.retry}
            />
            <footer className={styles.meta}>
              <time>{item.capturedAt}</time>
              <span>{item.size}</span>
            </footer>
          </>
        ) : (
          <div
            className={styles.status}
            role={preview.status === "error" || unavailable || missing ? "alert" : "status"}
          >
            {statusMessage}
          </div>
        )}
      </section>
    </div>
  );
}
