import { describe, expect, it } from "vitest";
import { parseQrConversionFeedback } from "../../src/app/qrClient";

describe("QR conversion feedback contract", () => {
  it("accepts the complete hotkey feedback payload", () => {
    expect(parseQrConversionFeedback({
      success: true,
      kind: "text_to_image",
      systemClipboardSynced: false,
      message: "结果已保存到历史",
      code: "system_clipboard_busy"
    })).toEqual({
      success: true,
      kind: "text_to_image",
      systemClipboardSynced: false,
      message: "结果已保存到历史",
      code: "system_clipboard_busy"
    });
  });

  it("rejects incomplete or unknown feedback instead of rendering it", () => {
    expect(parseQrConversionFeedback(null)).toBeNull();
    expect(parseQrConversionFeedback({
      success: true,
      kind: "file_to_image",
      systemClipboardSynced: true,
      message: "invalid"
    })).toBeNull();
  });
});
