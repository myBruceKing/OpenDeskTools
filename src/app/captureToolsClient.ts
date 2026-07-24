import { invoke } from "@tauri-apps/api/core";

export type ScreenshotCaptureResult = {
  status: "cancelled" | "copied" | "saved" | "pinned" | "qrDecoded";
  width: number | null;
  height: number | null;
  message: string;
};

export type PinImageResult = {
  pinId: string;
  width: number;
  height: number;
  message: string;
};

function record(value: unknown) {
  if (!value || typeof value !== "object") throw new Error("Invalid capture tool payload");
  return value as Record<string, unknown>;
}

export function parseScreenshotCaptureResult(value: unknown): ScreenshotCaptureResult {
  const payload = record(value);
  if (
    !["cancelled", "copied", "saved", "pinned", "qrDecoded"].includes(
      payload.status as string,
    )
    || (payload.width !== null && typeof payload.width !== "number")
    || (payload.height !== null && typeof payload.height !== "number")
    || typeof payload.message !== "string"
  ) {
    throw new Error("Invalid screenshot capture payload");
  }
  return {
    status: payload.status as ScreenshotCaptureResult["status"],
    width: payload.width,
    height: payload.height,
    message: payload.message
  };
}

export function parsePinImageResult(value: unknown): PinImageResult {
  const payload = record(value);
  if (
    typeof payload.pinId !== "string"
    || typeof payload.width !== "number"
    || typeof payload.height !== "number"
    || typeof payload.message !== "string"
  ) {
    throw new Error("Invalid pin image payload");
  }
  return {
    pinId: payload.pinId,
    width: payload.width,
    height: payload.height,
    message: payload.message
  };
}

export async function captureScreenshot() {
  return parseScreenshotCaptureResult(await invoke("capture_screenshot"));
}

export async function pinLatestImage() {
  return parsePinImageResult(await invoke("pin_latest_image"));
}
