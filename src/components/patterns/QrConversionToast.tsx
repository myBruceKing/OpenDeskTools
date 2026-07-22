import { useEffect, useState } from "react";
import { CheckmarkCircle20Regular, Warning20Regular } from "@fluentui/react-icons";
import { listenQrConversionFeedback, type QrConversionFeedback } from "../../app/qrClient";
import styles from "./QrConversionToast.module.css";

export function QrConversionToast() {
  const [feedback, setFeedback] = useState<QrConversionFeedback | null>(null);

  useEffect(() => {
    let closed = false;
    let unlisten: (() => void) | undefined;
    void listenQrConversionFeedback((next) => {
      if (!closed) setFeedback(next);
    }).then((cleanup) => {
      if (closed) cleanup();
      else unlisten = cleanup;
    });
    return () => {
      closed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (!feedback) return undefined;
    const timer = window.setTimeout(() => setFeedback(null), feedback.success ? 3200 : 4800);
    return () => window.clearTimeout(timer);
  }, [feedback]);

  if (!feedback) return null;
  return (
    <div className={styles.toast} role={feedback.success ? "status" : "alert"} aria-live="polite">
      {feedback.success ? <CheckmarkCircle20Regular aria-hidden="true" /> : <Warning20Regular aria-hidden="true" />}
      <span>{feedback.message}</span>
    </div>
  );
}
