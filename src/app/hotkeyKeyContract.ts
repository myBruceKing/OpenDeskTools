export const HOTKEY_MODIFIER_ORDER = ["Ctrl", "Alt", "Shift", "Win"] as const;

const CANONICAL_NAMED_KEYS = new Set([
  "Backquote",
  "Backslash",
  "BracketLeft",
  "BracketRight",
  "Pause",
  "Comma",
  "Equal",
  "Minus",
  "Period",
  "Quote",
  "Semicolon",
  "Slash",
  "Backspace",
  "CapsLock",
  "Enter",
  "Space",
  "Tab",
  "Delete",
  "End",
  "Home",
  "Insert",
  "PageDown",
  "PageUp",
  "PrintScreen",
  "ScrollLock",
  "ArrowDown",
  "ArrowLeft",
  "ArrowRight",
  "ArrowUp",
  "NumLock",
  "Numpad0",
  "Numpad1",
  "Numpad2",
  "Numpad3",
  "Numpad4",
  "Numpad5",
  "Numpad6",
  "Numpad7",
  "Numpad8",
  "Numpad9",
  "NumpadAdd",
  "NumpadDecimal",
  "NumpadDivide",
  "NumpadMultiply",
  "NumpadSubtract",
  "Escape",
  "AudioVolumeDown",
  "AudioVolumeUp",
  "AudioVolumeMute",
  "MediaPlay",
  "MediaPlayPause",
  "MediaStop",
  "MediaTrackNext",
  "MediaTrackPrevious"
]);

const LEGACY_KEY_ALIASES: Record<string, string> = {
  " ": "Space",
  Esc: "Escape",
  Up: "ArrowUp",
  Down: "ArrowDown",
  Left: "ArrowLeft",
  Right: "ArrowRight",
  "↑": "ArrowUp",
  "↓": "ArrowDown",
  "←": "ArrowLeft",
  "→": "ArrowRight",
  "`": "Backquote",
  "~": "Backquote",
  "\\": "Backslash",
  "[": "BracketLeft",
  "]": "BracketRight",
  ",": "Comma",
  "=": "Equal",
  "-": "Minus",
  ".": "Period",
  "'": "Quote",
  ";": "Semicolon",
  "/": "Slash"
};

export function isCanonicalHotkeyKey(value: string): boolean {
  if (/^[A-Z0-9]$/.test(value) || CANONICAL_NAMED_KEYS.has(value)) {
    return true;
  }
  return /^F([1-9]|1\d|2[0-4])$/.test(value);
}

export function canonicalHotkeyKeyFromInput(code: string, key: string): string | null {
  // RegisterHotKey cannot distinguish the extended variants represented by
  // these dependency codes. Normalize honest aliases and reject the Windows
  // backend's unsafe NumpadEqual -> E mapping.
  if (code === "NumpadEnter") {
    return "Enter";
  }
  if (code === "MediaPause") {
    return "Pause";
  }
  if (code === "NumpadEqual") {
    return null;
  }
  if (/^Key[A-Z]$/.test(code)) {
    return code.slice(3);
  }
  if (/^Digit[0-9]$/.test(code)) {
    return code.slice(5);
  }
  if (isCanonicalHotkeyKey(code)) {
    return code;
  }

  const aliased = LEGACY_KEY_ALIASES[key] ?? key;
  const normalized = aliased.length === 1 ? aliased.toUpperCase() : aliased;
  return isCanonicalHotkeyKey(normalized) ? normalized : null;
}
