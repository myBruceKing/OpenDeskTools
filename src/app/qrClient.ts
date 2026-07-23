import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type QrConversionResult = {
  kind: "text_to_image" | "image_to_text";
  systemClipboardSynced: boolean;
  message: string;
};

export type QrConversionFeedback = {
  success: boolean;
  kind: QrConversionResult["kind"] | null;
  systemClipboardSynced: boolean;
  message: string;
  code?: string;
};

export function parseQrConversionFeedback(value: unknown): QrConversionFeedback | null {
  if (!value || typeof value !== "object") return null;
  const payload = value as Record<string, unknown>;
  if (
    typeof payload.success !== "boolean"
    || typeof payload.systemClipboardSynced !== "boolean"
    || typeof payload.message !== "string"
    || (payload.kind !== null && payload.kind !== "text_to_image" && payload.kind !== "image_to_text")
  ) return null;
  return {
    success: payload.success,
    kind: payload.kind,
    systemClipboardSynced: payload.systemClipboardSynced,
    message: payload.message,
    code: typeof payload.code === "string" ? payload.code : undefined
  };
}

function result(value: unknown): QrConversionResult {
  if (!value || typeof value !== "object") throw new Error("Invalid QR conversion payload");
  const payload = value as Record<string, unknown>;
  if (
    (payload.kind !== "text_to_image" && payload.kind !== "image_to_text")
    || typeof payload.systemClipboardSynced !== "boolean"
    || typeof payload.message !== "string"
  ) throw new Error("Invalid QR conversion payload");
  return {
    kind: payload.kind,
    systemClipboardSynced: payload.systemClipboardSynced,
    message: payload.message
  };
}

export async function convertLatestInternalClipboardQr() {
  return result(await invoke("convert_latest_clipboard_qr"));
}

export async function listenQrConversionFeedback(listener: (feedback: QrConversionFeedback) => void) {
  return listen<unknown>("qr://conversion-result", (event) => {
    const feedback = parseQrConversionFeedback(event.payload);
    if (feedback) listener(feedback);
  });
}
