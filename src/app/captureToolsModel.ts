import { useState } from "react";
import { captureScreenshot, pinLatestImage } from "./captureToolsClient";

type CaptureToolAction = "screenshot" | "pin";

function messageFor(error: unknown) {
  return error && typeof error === "object" && typeof (error as { message?: unknown }).message === "string"
    ? (error as { message: string }).message
    : "操作失败，请重试。";
}

export function useCaptureTools() {
  const [pending, setPending] = useState<CaptureToolAction | null>(null);
  const [message, setMessage] = useState<{ action: CaptureToolAction; text: string } | null>(null);

  const run = async (action: CaptureToolAction) => {
    if (pending !== null) return;
    setPending(action);
    try {
      const result = action === "screenshot"
        ? await captureScreenshot()
        : await pinLatestImage();
      setMessage({ action, text: result.message });
    } catch (error: unknown) {
      setMessage({ action, text: messageFor(error) });
    } finally {
      setPending(null);
    }
  };

  return {
    pending,
    message,
    startScreenshot: () => run("screenshot"),
    pinLatest: () => run("pin")
  };
}
