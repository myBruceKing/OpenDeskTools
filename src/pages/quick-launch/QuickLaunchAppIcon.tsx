import { useEffect, useRef, useState } from "react";
import { quickLaunchClient } from "../../app/quickLaunchClient";
import type { QuickLaunchApp } from "../../app/quickLaunchModel";
import { AppIcon } from "../../components/patterns/ToolMenuPreview";
import styles from "./QuickLaunchAppIcon.module.css";

export function QuickLaunchAppIcon({ app }: { app: QuickLaunchApp }) {
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
        // The shared AppIcon fallback remains visible when Windows cannot
        // extract an icon for this program.
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
    <span className={styles.host} ref={host}>
      <AppIcon src={iconSrc} label={`${app.name} 图标`} size="row" />
    </span>
  );
}
