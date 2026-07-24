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
      message: "ok"
    })).toEqual({
      status: "copied",
      width: 320,
      height: 200,
      message: "ok"
    });
    expect(parseScreenshotCaptureResult({
      status: "cancelled",
      width: null,
      height: null,
      message: "cancelled"
    }).status).toBe("cancelled");
    for (const status of ["saved", "pinned", "qrDecoded"] as const) {
      expect(parseScreenshotCaptureResult({
        status,
        width: 320,
        height: 200,
        message: "ok"
      }).status).toBe(status);
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
  });
});
