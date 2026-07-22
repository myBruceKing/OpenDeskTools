import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const HOTKEY_CAPTURE_EVENT = "hotkey://capture-token";
const SESSION_ID_PATTERN = /^hotkey-capture-[1-9]\d*$/;
const MODIFIER_ORDER = ["Ctrl", "Alt", "Shift", "Win"] as const;
const NAMED_KEYS = new Set([
  "Backspace",
  "Enter",
  "Space",
  "PageUp",
  "PageDown",
  "End",
  "Home",
  "ArrowLeft",
  "ArrowUp",
  "ArrowRight",
  "ArrowDown",
  "PrintScreen",
  "Insert",
  "Delete",
  "Backquote"
]);

type InvokeFunction = (command: string, args?: Record<string, unknown>) => Promise<unknown>;
type ListenFunction = (
  event: string,
  handler: (event: { payload: unknown }) => void
) => Promise<() => void>;

export type HotkeyCaptureSession = {
  sessionId: string;
};

export type HotkeyCaptureStopResult = {
  sessionId: string;
  stopped: boolean;
};

export type HotkeyCaptureTokenEvent = {
  sessionId: string;
  token: string;
};

export type HotkeyCaptureClient = {
  start: () => Promise<HotkeyCaptureSession>;
  stop: (sessionId: string) => Promise<HotkeyCaptureStopResult>;
  subscribe: (listener: (event: HotkeyCaptureTokenEvent) => void) => Promise<() => void>;
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function parseSessionId(value: unknown): string {
  if (typeof value !== "string" || !SESSION_ID_PATTERN.test(value)) {
    throw new Error("Invalid hotkey capture payload field: sessionId");
  }
  return value;
}

function isNormalizedKey(value: string): boolean {
  if (/^[A-Z0-9]$/.test(value) || NAMED_KEYS.has(value)) {
    return true;
  }
  const functionKey = /^F([1-9]|1\d|2[0-4])$/.exec(value);
  return functionKey !== null;
}

function isNormalizedToken(value: string): boolean {
  const parts = value.split("+");
  const key = parts.pop();
  if (!key || !isNormalizedKey(key)) {
    return false;
  }
  let previousModifierIndex = -1;
  for (const modifier of parts) {
    const modifierIndex = MODIFIER_ORDER.indexOf(
      modifier as (typeof MODIFIER_ORDER)[number]
    );
    if (modifierIndex <= previousModifierIndex) {
      return false;
    }
    previousModifierIndex = modifierIndex;
  }
  return parts.length > 0 && parts.includes("Win");
}

export function parseHotkeyCaptureSession(value: unknown): HotkeyCaptureSession {
  if (!isRecord(value)) {
    throw new Error("Invalid hotkey capture session payload");
  }
  return { sessionId: parseSessionId(value.sessionId) };
}

export function parseHotkeyCaptureStopResult(value: unknown): HotkeyCaptureStopResult {
  if (!isRecord(value) || typeof value.stopped !== "boolean") {
    throw new Error("Invalid hotkey capture stop payload");
  }
  return {
    sessionId: parseSessionId(value.sessionId),
    stopped: value.stopped
  };
}

export function parseHotkeyCaptureTokenEvent(value: unknown): HotkeyCaptureTokenEvent {
  if (!isRecord(value)) {
    throw new Error("Invalid hotkey capture event payload");
  }
  const sessionId = parseSessionId(value.sessionId);
  if (typeof value.token !== "string" || !isNormalizedToken(value.token)) {
    throw new Error("Invalid hotkey capture payload field: token");
  }
  return { sessionId, token: value.token };
}

export function createHotkeyCaptureClient({
  invokeFunction = invoke as InvokeFunction,
  listenFunction = listen as ListenFunction
}: {
  invokeFunction?: InvokeFunction;
  listenFunction?: ListenFunction;
} = {}): HotkeyCaptureClient {
  return {
    async start() {
      return parseHotkeyCaptureSession(await invokeFunction("start_hotkey_capture"));
    },

    async stop(sessionId) {
      const validSessionId = parseSessionId(sessionId);
      const result = parseHotkeyCaptureStopResult(
        await invokeFunction("stop_hotkey_capture", { sessionId: validSessionId })
      );
      if (result.sessionId !== validSessionId) {
        throw new Error("Hotkey capture stop response used a different sessionId");
      }
      return result;
    },

    subscribe(listener) {
      return listenFunction(HOTKEY_CAPTURE_EVENT, (event) => {
        try {
          const payload = parseHotkeyCaptureTokenEvent(event.payload);
          listener(payload);
        } catch (error) {
          console.error("Ignoring an invalid hotkey capture event", error);
        }
      });
    }
  };
}

export const hotkeyCaptureClient = createHotkeyCaptureClient();
