import { useState } from "react";
import { convertLatestInternalClipboardQr } from "./qrClient";

function messageFor(error: unknown) {
  return error && typeof error === "object" && typeof (error as { message?: unknown }).message === "string"
    ? (error as { message: string }).message
    : "二维码转换失败，请重试。";
}

export function useQrConversion() {
  const [pending, setPending] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const convertLatest = async () => {
    if (pending) return;
    setPending(true);
    try {
      const result = await convertLatestInternalClipboardQr();
      setMessage(result.message);
    } catch (error: unknown) {
      setMessage(messageFor(error));
    } finally {
      setPending(false);
    }
  };
  return { pending, message, convertLatest };
}
