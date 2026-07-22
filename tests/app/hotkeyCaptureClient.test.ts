import { describe, expect, it, vi } from "vitest";
import {
  createHotkeyCaptureClient,
  parseHotkeyCaptureSession,
  parseHotkeyCaptureStopResult,
  parseHotkeyCaptureTokenEvent
} from "../../src/app/hotkeyCaptureClient";

describe("hotkeyCaptureClient", () => {
  it("invokes start and idempotent stop with strict session payloads", async () => {
    const invokeFunction = vi.fn(async (command: string) => {
      if (command === "start_hotkey_capture") {
        return { sessionId: "hotkey-capture-1" };
      }
      return { sessionId: "hotkey-capture-1", stopped: true };
    });
    const client = createHotkeyCaptureClient({ invokeFunction });

    await expect(client.start()).resolves.toEqual({ sessionId: "hotkey-capture-1" });
    await expect(client.stop("hotkey-capture-1")).resolves.toEqual({
      sessionId: "hotkey-capture-1",
      stopped: true
    });
    expect(invokeFunction).toHaveBeenNthCalledWith(1, "start_hotkey_capture");
    expect(invokeFunction).toHaveBeenNthCalledWith(2, "stop_hotkey_capture", {
      sessionId: "hotkey-capture-1"
    });
  });

  it("strictly validates sessions, stop results, and normalized Win tokens", () => {
    expect(parseHotkeyCaptureSession({ sessionId: "hotkey-capture-7" })).toEqual({
      sessionId: "hotkey-capture-7"
    });
    expect(
      parseHotkeyCaptureStopResult({ sessionId: "hotkey-capture-7", stopped: false })
    ).toEqual({ sessionId: "hotkey-capture-7", stopped: false });
    expect(
      parseHotkeyCaptureTokenEvent({
        sessionId: "hotkey-capture-7",
        token: "Shift+Win+S"
      })
    ).toEqual({ sessionId: "hotkey-capture-7", token: "Shift+Win+S" });
    expect(
      parseHotkeyCaptureTokenEvent({
        sessionId: "hotkey-capture-7",
        token: "Win+Backquote"
      })
    ).toEqual({ sessionId: "hotkey-capture-7", token: "Win+Backquote" });
    for (const token of [
      "Win+2",
      "Ctrl+Win+Numpad2",
      "Shift+Win+Minus",
      "Win+AudioVolumeUp",
      "Win+MediaPlayPause"
    ]) {
      expect(parseHotkeyCaptureTokenEvent({
        sessionId: "hotkey-capture-7",
        token
      })).toEqual({ sessionId: "hotkey-capture-7", token });
    }

    expect(() => parseHotkeyCaptureSession({ sessionId: "stale" })).toThrow();
    expect(() =>
      parseHotkeyCaptureStopResult({ sessionId: "hotkey-capture-7", stopped: "yes" })
    ).toThrow();
    for (const token of ["Win + V", "Win+Win+V", "Win+Shift+S", "Ctrl+V", "F1 F2"]) {
      expect(() =>
        parseHotkeyCaptureTokenEvent({ sessionId: "hotkey-capture-7", token })
      ).toThrow();
    }
  });

  it("subscribes before start and delivers strict session-tagged events", async () => {
    let eventHandler: ((event: { payload: unknown }) => void) | undefined;
    const unlisten = vi.fn();
    const listenFunction = vi.fn(async (_event, handler) => {
      eventHandler = handler;
      return unlisten;
    });
    const listener = vi.fn();
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => undefined);
    const client = createHotkeyCaptureClient({ listenFunction });

    await expect(client.subscribe(listener)).resolves.toBe(unlisten);
    expect(listenFunction).toHaveBeenCalledWith("hotkey://capture-token", expect.any(Function));

    eventHandler?.({ payload: { sessionId: "hotkey-capture-1", token: "Win+V" } });
    eventHandler?.({ payload: { sessionId: "hotkey-capture-2", token: "Win+V" } });
    eventHandler?.({ payload: { sessionId: "hotkey-capture-2", token: "bad token" } });

    expect(listener).toHaveBeenCalledTimes(2);
    expect(listener).toHaveBeenNthCalledWith(1, {
      sessionId: "hotkey-capture-1",
      token: "Win+V"
    });
    expect(listener).toHaveBeenNthCalledWith(2, {
      sessionId: "hotkey-capture-2",
      token: "Win+V"
    });
    expect(consoleError).toHaveBeenCalledTimes(1);
    consoleError.mockRestore();
  });

  it("rejects a mismatched stop response and never accepts a stale replacement", async () => {
    const client = createHotkeyCaptureClient({
      invokeFunction: async () => ({ sessionId: "hotkey-capture-9", stopped: true })
    });

    await expect(client.stop("hotkey-capture-8")).rejects.toThrow(
      "different sessionId"
    );
  });
});
