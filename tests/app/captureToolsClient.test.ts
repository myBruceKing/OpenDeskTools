import { describe, expect, it } from "vitest";
import {
  parsePinImageResult,
  parseScreenshotCaptureResult
} from "../../src/app/captureToolsClient";

describe("capture tools client contracts", () => {
  it("parses every screenshot toolbar outcome", () => {
    expect(parseScreenshotCaptureResult({
      status: "copied",
      width: 320,
      height: 200,
      historyStatus: "retained",
      message: "ok"
    })).toEqual({
      status: "copied",
      width: 320,
      height: 200,
      historyStatus: "retained",
      message: "ok"
    });
    expect(parseScreenshotCaptureResult({
      status: "cancelled",
      width: null,
      height: null,
      historyStatus: "notAttempted",
      message: "cancelled"
    }).status).toBe("cancelled");
    for (const status of ["saved", "pinned", "qrDecoded"] as const) {
      expect(parseScreenshotCaptureResult({
        status,
        width: 320,
        height: 200,
        historyStatus: "failed",
        message: "ok"
      }).status).toBe(status);
    }
    expect(() => parseScreenshotCaptureResult({
      status: "copied",
      width: 320,
      height: 200,
      historyStatus: "unknown",
      message: "bad"
    })).toThrow("Invalid screenshot capture payload");
    for (const payload of [
      {
        status: "cancelled",
        width: 1,
        height: 1,
        historyStatus: "notAttempted",
        message: "bad"
      },
      {
        status: "copied",
        width: 0,
        height: 200,
        historyStatus: "retained",
        message: "bad"
      },
      {
        status: "saved",
        width: 320,
        height: 200,
        historyStatus: "notAttempted",
        message: "bad"
      }
    ]) {
      expect(() => parseScreenshotCaptureResult(payload)).toThrow(
        "Invalid screenshot capture payload"
      );
    }
  });

  it("keeps pin ids as decimal strings and rejects malformed payloads", () => {
    expect(parsePinImageResult({
      pinId: "12",
      width: 640,
      height: 480,
      message: "ok"
    }).pinId).toBe("12");
    expect(() => parsePinImageResult({
      pinId: 12,
      width: 640,
      height: 480,
      message: "bad"
    })).toThrow("Invalid pin image payload");
    expect(() => parsePinImageResult({
      pinId: "not-a-decimal-id",
      width: 640,
      height: 480,
      message: "bad"
    })).toThrow("Invalid pin image payload");
  });
});
