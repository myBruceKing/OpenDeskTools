import { useCallback, useEffect, useRef, useState } from "react";
import { normalizeClipboardCommandError } from "../../app/clipboardModel";
import type { ImagePreviewState } from "../../components/patterns/ImagePreview";

export type LoadClipboardImage = (id: string) => Promise<Blob>;

const IDLE_STATE: ImagePreviewState = {
  status: "idle",
  itemId: null,
  url: null,
  message: "选择图片记录后查看预览。",
  retryable: false
};

function errorCode(value: unknown) {
  if (typeof value === "object" && value !== null && "code" in value) {
    const code = (value as { code?: unknown }).code;
    return typeof code === "string" ? code : null;
  }
  return null;
}

function imageErrorState(itemId: string, error: unknown): ImagePreviewState {
  const code = errorCode(error);
  if (code === "clipboard_image_too_large" || code === "clipboard_image_oversized") {
    return {
      status: "oversized",
      itemId,
      url: null,
      message: "图片过大，无法安全预览。",
      retryable: false
    };
  }
  if (
    code === "clipboard_image_unavailable"
    || code === "clipboard_content_unavailable"
    || code === "clipboard_item_not_found"
  ) {
    return {
      status: "unavailable",
      itemId,
      url: null,
      message: "图片内容不可用。",
      retryable: false
    };
  }

  const normalized = normalizeClipboardCommandError(error);
  return {
    status: "error",
    itemId,
    url: null,
    message: normalized.retryable ? "图片加载失败，请重试。" : "图片内容不可用。",
    retryable: normalized.retryable
  };
}

export function useClipboardImagePreview(
  itemId: string | null,
  loadImage: LoadClipboardImage
) {
  const [state, setState] = useState<ImagePreviewState>(IDLE_STATE);
  const [attempt, setAttempt] = useState(0);
  const requestToken = useRef(0);
  const currentUrl = useRef<string | null>(null);
  const mounted = useRef(true);

  const revokeCurrentUrl = useCallback(() => {
    const url = currentUrl.current;
    if (url === null) {
      return;
    }
    currentUrl.current = null;
    URL.revokeObjectURL(url);
  }, []);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      requestToken.current += 1;
      revokeCurrentUrl();
    };
  }, [revokeCurrentUrl]);

  useEffect(() => {
    const token = ++requestToken.current;
    revokeCurrentUrl();

    if (itemId === null) {
      setState(IDLE_STATE);
      return undefined;
    }

    setState({
      status: "loading",
      itemId,
      url: null,
      message: "正在加载图片预览…",
      retryable: false
    });

    void (async () => {
      try {
        const blob = await loadImage(itemId);
        if (!mounted.current || token !== requestToken.current) {
          return;
        }
        if (blob.size === 0) {
          setState({
            status: "unavailable",
            itemId,
            url: null,
            message: "图片内容为空，无法预览。",
            retryable: false
          });
          return;
        }

        const url = URL.createObjectURL(blob);
        if (!mounted.current || token !== requestToken.current) {
          URL.revokeObjectURL(url);
          return;
        }
        currentUrl.current = url;
        setState({
          status: "decoding",
          itemId,
          url,
          message: "正在解码图片…",
          retryable: false
        });
      } catch (error: unknown) {
        if (mounted.current && token === requestToken.current) {
          setState(imageErrorState(itemId, error));
        }
      }
    })();

    return () => {
      requestToken.current += 1;
      revokeCurrentUrl();
    };
  }, [attempt, itemId, loadImage, revokeCurrentUrl]);

  const markLoaded = useCallback((url: string) => {
    if (currentUrl.current !== url) {
      return;
    }
    setState((current) => current.url === url
      ? { ...current, status: "ready", message: "" }
      : current);
  }, []);

  const markDecodeError = useCallback((url: string) => {
    if (currentUrl.current !== url) {
      return;
    }
    revokeCurrentUrl();
    setState((current) => current.url === url
      ? {
          status: "decode-error",
          itemId: current.itemId,
          url: null,
          message: "图片无法解码，请重试。",
          retryable: true
        }
      : current);
  }, [revokeCurrentUrl]);

  const retry = useCallback(() => {
    if (itemId !== null) {
      setAttempt((current) => current + 1);
    }
  }, [itemId]);

  const release = useCallback(() => {
    requestToken.current += 1;
    revokeCurrentUrl();
    if (mounted.current && itemId !== null) {
      setState({
        status: "unavailable",
        itemId,
        url: null,
        message: "图片预览已释放，可重新加载。",
        retryable: true
      });
    }
  }, [itemId, revokeCurrentUrl]);

  const presentedState: ImagePreviewState = itemId === null
    ? IDLE_STATE
    : state.itemId === itemId
      ? state
      : {
          status: "loading",
          itemId,
          url: null,
          message: "正在加载图片预览…",
          retryable: false
        };

  return {
    state: presentedState,
    markLoaded,
    markDecodeError,
    retry,
    release
  };
}
