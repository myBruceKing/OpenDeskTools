import { invoke } from "@tauri-apps/api/core";

export type ScreenshotCaptureResult = {
  status: "cancelled" | "copied" | "saved" | "pinned" | "qrDecoded";
  width: number | null;
  height: number | null;
  historyStatus: "notAttempted" | "retained" | "notRetained" | "failed";
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

function isPositiveSafeInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value > 0;
}

export function parseScreenshotCaptureResult(value: unknown): ScreenshotCaptureResult {
  const payload = record(value);
  const status = payload.status as ScreenshotCaptureResult["status"];
  const historyStatus = payload.historyStatus as ScreenshotCaptureResult["historyStatus"];
  const cancelled = status === "cancelled";
  if (
    !["cancelled", "copied", "saved", "pinned", "qrDecoded"].includes(
      status,
    )
    || !["notAttempted", "retained", "notRetained", "failed"].includes(
      historyStatus,
    )
    || typeof payload.message !== "string"
    || (
      cancelled
        ? payload.width !== null
          || payload.height !== null
          || historyStatus !== "notAttempted"
        : !isPositiveSafeInteger(payload.width)
          || !isPositiveSafeInteger(payload.height)
          || historyStatus === "notAttempted"
    )
  ) {
    throw new Error("Invalid screenshot capture payload");
  }
  return {
    status,
    width: payload.width as number | null,
    height: payload.height as number | null,
    historyStatus,
    message: payload.message
  };
}

export function parsePinImageResult(value: unknown): PinImageResult {
  const payload = record(value);
  if (
    typeof payload.pinId !== "string"
    || !/^(?:0|[1-9]\d*)$/.test(payload.pinId)
    || !isPositiveSafeInteger(payload.width)
    || !isPositiveSafeInteger(payload.height)
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
