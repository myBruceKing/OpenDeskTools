import { describe, expect, it } from "vitest";
import {
  canonicalHotkeyKeyFromInput,
  isCanonicalHotkeyKey
} from "../../src/app/hotkeyKeyContract";

const namedCodes = [
  "Backquote", "Backslash", "BracketLeft", "BracketRight", "Pause", "Comma",
  "Equal", "Minus", "Period", "Quote", "Semicolon", "Slash", "Backspace",
  "CapsLock", "Enter", "Space", "Tab", "Delete", "End", "Home", "Insert",
  "PageDown", "PageUp", "PrintScreen", "ScrollLock", "ArrowDown", "ArrowLeft",
  "ArrowRight", "ArrowUp", "NumLock", "Numpad0", "Numpad1", "Numpad2",
  "Numpad3", "Numpad4", "Numpad5", "Numpad6", "Numpad7", "Numpad8",
  "Numpad9", "NumpadAdd", "NumpadDecimal", "NumpadDivide",
  "NumpadMultiply", "NumpadSubtract", "Escape", "AudioVolumeDown",
  "AudioVolumeUp", "AudioVolumeMute", "MediaPlay", "MediaPlayPause",
  "MediaStop", "MediaTrackNext", "MediaTrackPrevious"
];

const physicalCodeCases = [
  ...Array.from({ length: 26 }, (_, index) => {
    const letter = String.fromCharCode(65 + index);
    return [`Key${letter}`, letter] as const;
  }),
  ...Array.from({ length: 10 }, (_, index) => [`Digit${index}`, String(index)] as const),
  ...Array.from({ length: 24 }, (_, index) => [`F${index + 1}`, `F${index + 1}`] as const),
  ...namedCodes.map((code) => [code, code] as const)
];

describe("hotkey physical key contract", () => {
  it("normalizes every global-hotkey code supported by the Windows backend", () => {
    expect(physicalCodeCases).toHaveLength(114);
    for (const [code, canonical] of physicalCodeCases) {
      expect(canonicalHotkeyKeyFromInput(code, "Unidentified"), code).toBe(canonical);
      expect(isCanonicalHotkeyKey(canonical), canonical).toBe(true);
    }
  });

  it("normalizes indistinguishable Windows keys and rejects the unsafe keypad-equal alias", () => {
    expect(canonicalHotkeyKeyFromInput("NumpadEnter", "Enter")).toBe("Enter");
    expect(canonicalHotkeyKeyFromInput("MediaPause", "MediaPause")).toBe("Pause");
    expect(canonicalHotkeyKeyFromInput("NumpadEqual", "=")).toBeNull();
  });

  it("uses physical codes instead of shifted or layout-dependent characters", () => {
    expect(canonicalHotkeyKeyFromInput("Digit2", "@")).toBe("2");
    expect(canonicalHotkeyKeyFromInput("Minus", "_")).toBe("Minus");
    expect(canonicalHotkeyKeyFromInput("BracketLeft", "{")).toBe("BracketLeft");
    expect(canonicalHotkeyKeyFromInput("Numpad2", "ArrowDown")).toBe("Numpad2");
  });

  it("keeps legacy event-key aliases compatible without accepting unknown keys", () => {
    expect(canonicalHotkeyKeyFromInput("", "↑")).toBe("ArrowUp");
    expect(canonicalHotkeyKeyFromInput("", "~")).toBe("Backquote");
    expect(canonicalHotkeyKeyFromInput("", "Unidentified")).toBeNull();
  });
});
