import { useEffect, useState } from "react";
import { QrConversionToast } from "../components/patterns/QrConversionToast";
import {
  parseQrConversionFeedback,
  type QrConversionFeedback
} from "./qrClient";
import { useWindowSurfaceRuntime } from "./useWindowSurfaceRuntime";

declare global {
  interface Window {
    __OPENDESK_QR_FEEDBACK?: unknown;
  }
}

export function QrToastSurfaceRoot() {
  const [feedback, setFeedback] = useState<QrConversionFeedback | null>(() =>
    parseQrConversionFeedback(window.__OPENDESK_QR_FEEDBACK)
  );
  useWindowSurfaceRuntime();

  useEffect(() => {
    const sync = () => {
      const next = parseQrConversionFeedback(window.__OPENDESK_QR_FEEDBACK);
      if (next) setFeedback(next);
    };
    window.addEventListener("opendesk-qr-feedback", sync);
    sync();
    return () => window.removeEventListener("opendesk-qr-feedback", sync);
  }, []);

  return <QrConversionToast feedback={feedback} presentation="surface" />;
}
