import { Image24Regular } from "@fluentui/react-icons";
import { Button } from "../primitives/Button";
import styles from "./ImagePreview.module.css";

export type ImagePreviewStatus =
  | "idle"
  | "loading"
  | "decoding"
  | "ready"
  | "oversized"
  | "unavailable"
  | "error"
  | "decode-error";

export type ImagePreviewState = {
  status: ImagePreviewStatus;
  itemId: string | null;
  url: string | null;
  message: string;
  retryable: boolean;
};

type ImagePreviewProps = {
  state: ImagePreviewState;
  alt: string;
  className?: string;
  onLoad: (url: string) => void;
  onError: (url: string) => void;
  onRetry: () => void;
};

export function ImagePreview({
  state,
  alt,
  className = "",
  onLoad,
  onError,
  onRetry
}: ImagePreviewProps) {
  const showImage = state.url !== null
    && (state.status === "decoding" || state.status === "ready");
  const showStatus = state.status !== "ready";
  const statusRole = state.status === "error" || state.status === "decode-error"
    ? "alert"
    : "status";

  return (
    <div
      className={[styles.imagePreview, className].filter(Boolean).join(" ")}
      data-image-preview-state={state.status}
      aria-busy={state.status === "loading" || state.status === "decoding"}
    >
      <div className={styles.imageStage}>
        {showImage && (
          <img
            className={styles.image}
            src={state.url ?? undefined}
            alt={alt}
            draggable={false}
            onLoad={() => state.url && onLoad(state.url)}
            onError={() => state.url && onError(state.url)}
          />
        )}
        {showStatus && (
          <div
            className={styles.status}
            role={statusRole}
            aria-live={statusRole === "status" ? "polite" : undefined}
            aria-atomic="true"
          >
            <Image24Regular aria-hidden="true" />
            <span>{state.message}</span>
            {state.retryable && (
              <Button size="inline" onClick={onRetry}>
                重试
              </Button>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
