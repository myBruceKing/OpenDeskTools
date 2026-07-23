import { useEffect, useState } from "react";
import { CheckmarkCircle20Regular, Warning20Regular } from "@fluentui/react-icons";
import { listenQrConversionFeedback, type QrConversionFeedback } from "../../app/qrClient";
import styles from "./QrConversionToast.module.css";

type QrConversionToastProps = {
  feedback?: QrConversionFeedback | null;
};

export function QrConversionToast({ feedback: controlledFeedback }: QrConversionToastProps) {
  const [localFeedback, setLocalFeedback] = useState<QrConversionFeedback | null>(null);
  const controlled = controlledFeedback !== undefined;
  const feedback = controlled ? controlledFeedback : localFeedback;

  useEffect(() => {
    if (controlled) return undefined;
    let closed = false;
    let unlisten: (() => void) | undefined;
    void listenQrConversionFeedback((next) => {
      if (!closed) setLocalFeedback(next);
    }).then((cleanup) => {
      if (closed) cleanup();
      else unlisten = cleanup;
    });
    return () => {
      closed = true;
      unlisten?.();
    };
  }, [controlled]);

  useEffect(() => {
    if (controlled || !feedback) return undefined;
    const timer = window.setTimeout(
      () => setLocalFeedback(null),
      feedback.success ? 3200 : 4800
    );
    return () => window.clearTimeout(timer);
  }, [controlled, feedback]);

  if (!feedback) return null;
  return (
    <div className={styles.toast} role={feedback.success ? "status" : "alert"} aria-live="polite">
      {feedback.success ? <CheckmarkCircle20Regular aria-hidden="true" /> : <Warning20Regular aria-hidden="true" />}
      <span>{feedback.message}</span>
    </div>
  );
}
