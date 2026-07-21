import { Folder24Regular, Image24Regular } from "@fluentui/react-icons";
import { useEffect, useRef, useState } from "react";
import type { ClipboardItemViewModel } from "../../app/clipboardModel";
import { ClipboardWithLinesIcon } from "../icons/ClipboardWithLinesIcon";
import styles from "./SourceAppIcon.module.css";

export type LoadClipboardSourceIcon = (id: string) => Promise<Blob>;

const sourceIconCache = new Map<string, Promise<Blob>>();
const maximumCachedIcons = 128;

function loadCachedIcon(key: string, id: string, loadIcon: LoadClipboardSourceIcon) {
  const cached = sourceIconCache.get(key);
  if (cached) {
    return cached;
  }
  if (sourceIconCache.size >= maximumCachedIcons) {
    const oldestKey = sourceIconCache.keys().next().value as string | undefined;
    if (oldestKey) {
      sourceIconCache.delete(oldestKey);
    }
  }
  const request = loadIcon(id).catch((error: unknown) => {
    sourceIconCache.delete(key);
    throw error;
  });
  sourceIconCache.set(key, request);
  return request;
}

export function SourceAppIcon({
  item,
  loadIcon,
  className = ""
}: {
  item: ClipboardItemViewModel;
  loadIcon: LoadClipboardSourceIcon;
  className?: string;
}) {
  const hostRef = useRef<HTMLSpanElement>(null);
  const [visible, setVisible] = useState(() => typeof IntersectionObserver === "undefined");
  const [url, setUrl] = useState<string | null>(null);

  useEffect(() => {
    if (visible || !item.sourceIconAvailable || typeof IntersectionObserver === "undefined") {
      return undefined;
    }
    const host = hostRef.current;
    if (!host) {
      return undefined;
    }
    const observer = new IntersectionObserver((entries) => {
      if (entries.some((entry) => entry.isIntersecting)) {
        setVisible(true);
        observer.disconnect();
      }
    }, { rootMargin: "32px" });
    observer.observe(host);
    return () => observer.disconnect();
  }, [item.sourceIconAvailable, visible]);

  useEffect(() => {
    if (!visible || !item.sourceIconAvailable) {
      setUrl(null);
      return undefined;
    }
    let active = true;
    let objectUrl: string | null = null;
    const key = `${item.id}:${item.revision}`;
    void loadCachedIcon(key, item.id, loadIcon)
      .then((blob) => {
        if (!active) {
          return;
        }
        objectUrl = URL.createObjectURL(blob);
        setUrl(objectUrl);
      })
      .catch(() => {
        if (active) {
          setUrl(null);
        }
      });
    return () => {
      active = false;
      if (objectUrl) {
        URL.revokeObjectURL(objectUrl);
      }
    };
  }, [item.id, item.revision, item.sourceIconAvailable, loadIcon, visible]);

  const FallbackIcon = item.kind === "image"
    ? Image24Regular
    : item.kind === "files"
      ? Folder24Regular
      : ClipboardWithLinesIcon;
  return (
    <span
      ref={hostRef}
      className={[styles.iconSlot, className].filter(Boolean).join(" ")}
      aria-hidden="true"
      data-source-icon={url ? "ready" : "fallback"}
    >
      {url ? <img src={url} alt="" draggable={false} /> : <FallbackIcon />}
    </span>
  );
}
