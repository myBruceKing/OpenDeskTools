// @vitest-environment jsdom

import { act, createRef, useState, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  useHotkeyCaptureSession
} from "../../src/app/useHotkeyCaptureSession";
import type {
  HotkeyCaptureClient,
  HotkeyCaptureTokenEvent
} from "../../src/app/hotkeyCaptureClient";
import {
  ShortcutCaptureField,
  type ShortcutCaptureFieldHandle
} from "../../src/components/primitives/Field";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function captureClient(
  start: HotkeyCaptureClient["start"] = async () => ({ sessionId: "hotkey-capture-1" }),
  subscribeReady: Promise<void> = Promise.resolve()
) {
  let listener: ((event: HotkeyCaptureTokenEvent) => void) | null = null;
  const unsubscribe = vi.fn();
  const client: HotkeyCaptureClient = {
    start: vi.fn(start),
    stop: vi.fn(async (sessionId) => ({ sessionId, stopped: true })),
    subscribe: vi.fn(async (nextListener) => {
      listener = nextListener;
      await subscribeReady;
      return unsubscribe;
    })
  };
  return {
    client,
    unsubscribe,
    emit(event: HotkeyCaptureTokenEvent) {
      listener?.(event);
    }
  };
}

function CaptureHarness({
  client,
  onAppendToken,
  autoFocus = false
}: {
  client: HotkeyCaptureClient;
  onAppendToken: (token: string) => void;
  autoFocus?: boolean;
}) {
  const [value, setValue] = useState("");
  const [stopResult, setStopResult] = useState<string>("");
  const fieldRef = createRef<ShortcutCaptureFieldHandle>();
  const capture = useHotkeyCaptureSession({
    client,
    onToken: (token) => fieldRef.current?.acceptNativeToken(token)
  });

  return (
    <>
      <ShortcutCaptureField
        ref={fieldRef}
        value={value}
        label="截图快捷键"
        onChange={setValue}
        onAppendToken={onAppendToken}
        onCaptureStart={capture.start}
        onCaptureStop={capture.stop}
        autoFocus={autoFocus}
      />
      <button type="button">外部按钮</button>
      <button
        type="button"
        onClick={() => void capture.stop().then((stopped) => setStopResult(String(stopped)))}
      >
        重复停止
      </button>
      <output>{capture.status}</output>
      <output aria-label="停止结果">{stopResult}</output>
      {capture.message && <div role="status">{capture.message}</div>}
    </>
  );
}

describe("useHotkeyCaptureSession", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
    document.body.replaceChildren();
    vi.restoreAllMocks();
  });

  it("starts on capture focus, accepts only the active session, and stops on blur", async () => {
    const native = captureClient();
    const onAppendToken = vi.fn();
    await render(root, <CaptureHarness client={native.client} onAppendToken={onAppendToken} />);

    const capture = document.querySelector<HTMLElement>("[role='group']")!;
    const outside = document.querySelector<HTMLButtonElement>("button")!;
    await act(async () => capture.focus());
    await flush();
    expect(native.client.start).toHaveBeenCalledOnce();
    expect(document.querySelector("output")?.textContent).toBe("active");

    act(() => native.emit({ sessionId: "hotkey-capture-2", token: "Alt+S" }));
    expect(onAppendToken).not.toHaveBeenCalled();
    act(() => native.emit({ sessionId: "hotkey-capture-1", token: "Shift+Win+S" }));
    expect(onAppendToken).toHaveBeenCalledWith("Shift+Win+S");

    await act(async () => outside.focus());
    expect(native.client.stop).toHaveBeenCalledWith("hotkey-capture-1");
    act(() => native.emit({ sessionId: "hotkey-capture-1", token: "F2" }));
    expect(onAppendToken).toHaveBeenCalledOnce();
  });

  it("retries a rejected blur stop and remains idle when the retry succeeds", async () => {
    const native = captureClient();
    vi.mocked(native.client.stop)
      .mockRejectedValueOnce(new Error("transient stop failure"))
      .mockResolvedValue({ sessionId: "hotkey-capture-1", stopped: true });
    await render(root, <CaptureHarness client={native.client} onAppendToken={vi.fn()} />);

    const capture = document.querySelector<HTMLElement>("[role='group']")!;
    const outside = document.querySelector<HTMLButtonElement>("button")!;
    await act(async () => capture.focus());
    await flush();
    await act(async () => outside.focus());

    await vi.waitFor(() => expect(native.client.stop).toHaveBeenCalledTimes(2));
    expect(document.querySelector("output")?.textContent).toBe("idle");
    expect(document.querySelector("[role='status']")).toBeNull();
  });

  it("treats stopped false as an idempotent safe stop without retrying or warning", async () => {
    const native = captureClient();
    vi.mocked(native.client.stop).mockResolvedValue({
      sessionId: "hotkey-capture-1",
      stopped: false
    });
    await render(root, <CaptureHarness client={native.client} onAppendToken={vi.fn()} />);

    const capture = document.querySelector<HTMLElement>("[role='group']")!;
    const outside = document.querySelector<HTMLButtonElement>("button")!;
    await act(async () => capture.focus());
    await flush();
    await act(async () => outside.focus());
    await flush();

    expect(native.client.stop).toHaveBeenCalledOnce();
    expect(document.querySelector("output")?.textContent).toBe("idle");
    expect(document.querySelector("[role='status']")).toBeNull();
  });

  it("reports a visible fallback when every bounded stop attempt fails", async () => {
    const native = captureClient();
    vi.mocked(native.client.stop).mockRejectedValue(new Error("persistent stop failure"));
    await render(root, <CaptureHarness client={native.client} onAppendToken={vi.fn()} />);

    const capture = document.querySelector<HTMLElement>("[role='group']")!;
    const outside = document.querySelector<HTMLButtonElement>("button")!;
    await act(async () => capture.focus());
    await flush();
    await act(async () => outside.focus());
    const repeatedStop = Array.from(document.querySelectorAll("button")).find(
      (button) => button.textContent === "重复停止"
    )!;
    await act(async () => repeatedStop.click());

    await vi.waitFor(() => expect(native.client.stop).toHaveBeenCalledTimes(3));
    await vi.waitFor(() => {
      expect(document.querySelector("output")?.textContent).toBe("fallback");
    });
    expect(document.querySelector("[role='status']")?.textContent).toBe(
      "系统组合捕获未停止，请重试。"
    );
    expect(document.querySelector("[aria-label='停止结果']")?.textContent).toBe("false");
  });

  it("waits for the global listener before starting the native session", async () => {
    const listenerReady = deferred<void>();
    const native = captureClient(undefined, listenerReady.promise);
    await render(root, <CaptureHarness client={native.client} onAppendToken={vi.fn()} />);

    const capture = document.querySelector<HTMLElement>("[role='group']")!;
    await act(async () => capture.focus());
    await flush();
    expect(document.querySelector("output")?.textContent).toBe("starting");
    expect(native.client.start).not.toHaveBeenCalled();

    await act(async () => listenerReady.resolve());
    await flush();
    expect(native.client.start).toHaveBeenCalledOnce();
    expect(document.querySelector("output")?.textContent).toBe("active");
  });

  it("establishes listener readiness before the field autoFocus starts the first session", async () => {
    const native = captureClient();
    await render(
      root,
      <CaptureHarness client={native.client} onAppendToken={vi.fn()} autoFocus />
    );
    await flushAnimationFrame();
    await flush();

    expect(document.activeElement).toBe(document.querySelector("[role='group']"));
    expect(native.client.subscribe).toHaveBeenCalledOnce();
    expect(native.client.start).toHaveBeenCalledOnce();
    expect(document.querySelector("output")?.textContent).toBe("active");
    expect(document.querySelector("[role='status']")).toBeNull();
  });

  it("does not start a native session when the global listener fails", async () => {
    const listenerFailure = deferred<void>();
    const native = captureClient(undefined, listenerFailure.promise);
    await render(root, <CaptureHarness client={native.client} onAppendToken={vi.fn()} />);

    const capture = document.querySelector<HTMLElement>("[role='group']")!;
    await act(async () => capture.focus());
    await act(async () => listenerFailure.reject(new Error("listen unavailable")));
    await flush();

    expect(native.client.start).not.toHaveBeenCalled();
    expect(document.querySelector("[role='status']")?.textContent).toBe(
      "系统组合捕获不可用；普通按键仍可录入。"
    );
  });

  it("stops a session that resolves after the capture has already blurred", async () => {
    const request = deferred<{ sessionId: string }>();
    const native = captureClient(() => request.promise);
    await render(root, <CaptureHarness client={native.client} onAppendToken={vi.fn()} />);

    const capture = document.querySelector<HTMLElement>("[role='group']")!;
    const outside = document.querySelector<HTMLButtonElement>("button")!;
    await act(async () => capture.focus());
    await act(async () => outside.focus());
    const repeatedStop = Array.from(document.querySelectorAll("button")).find(
      (button) => button.textContent === "重复停止"
    )!;
    await act(async () => repeatedStop.click());
    expect(native.client.stop).not.toHaveBeenCalled();
    await act(async () => request.resolve({ sessionId: "hotkey-capture-3" }));
    await flush();

    expect(native.client.stop).toHaveBeenCalledWith("hotkey-capture-3");
    expect(native.client.stop).toHaveBeenCalledOnce();
    expect(document.querySelector("output")?.textContent).toBe("idle");
    expect(document.querySelector("[aria-label='停止结果']")?.textContent).toBe("true");
  });

  it("keeps a refocused replacement session active while the old stop finishes", async () => {
    const oldStop = deferred<{ sessionId: string; stopped: boolean }>();
    const native = captureClient();
    vi.mocked(native.client.start)
      .mockResolvedValueOnce({ sessionId: "hotkey-capture-1" })
      .mockResolvedValueOnce({ sessionId: "hotkey-capture-2" });
    vi.mocked(native.client.stop).mockImplementation((sessionId) => {
      if (sessionId === "hotkey-capture-1") {
        return oldStop.promise;
      }
      return Promise.resolve({ sessionId, stopped: true });
    });
    const onAppendToken = vi.fn();
    await render(root, <CaptureHarness client={native.client} onAppendToken={onAppendToken} />);

    const capture = document.querySelector<HTMLElement>("[role='group']")!;
    const outside = document.querySelector<HTMLButtonElement>("button")!;
    await act(async () => capture.focus());
    await flush();
    await act(async () => outside.focus());
    await act(async () => capture.focus());
    await flush();
    expect(document.querySelector("output")?.textContent).toBe("active");

    await act(async () => oldStop.resolve({ sessionId: "hotkey-capture-1", stopped: true }));
    await flush();
    act(() => native.emit({ sessionId: "hotkey-capture-1", token: "Win+V" }));
    act(() => native.emit({ sessionId: "hotkey-capture-2", token: "Win+V" }));

    expect(native.client.stop).toHaveBeenCalledWith("hotkey-capture-1");
    expect(document.querySelector("output")?.textContent).toBe("active");
    expect(onAppendToken).toHaveBeenCalledOnce();
    expect(onAppendToken).toHaveBeenCalledWith("Win+V");
  });

  it("shows a short fallback status while ordinary WebView capture remains usable", async () => {
    const native = captureClient(async () => Promise.reject(new Error("native unavailable")));
    const onAppendToken = vi.fn();
    await render(root, <CaptureHarness client={native.client} onAppendToken={onAppendToken} />);

    const capture = document.querySelector<HTMLElement>("[role='group']")!;
    await act(async () => capture.focus());
    await flush();
    expect(document.querySelector("[role='status']")?.textContent).toBe(
      "系统组合捕获不可用；普通按键仍可录入。"
    );

    act(() => {
      capture.dispatchEvent(new KeyboardEvent("keydown", { key: "F2", bubbles: true }));
    });
    expect(onAppendToken).toHaveBeenCalledWith("F2");
  });

  it("unsubscribes and stops the active native session on page unmount", async () => {
    const native = captureClient();
    await render(root, <CaptureHarness client={native.client} onAppendToken={vi.fn()} />);
    const capture = document.querySelector<HTMLElement>("[role='group']")!;
    await act(async () => capture.focus());
    await flush();

    await act(async () => root.unmount());
    expect(native.unsubscribe).toHaveBeenCalledOnce();
    expect(native.client.stop).toHaveBeenCalledWith("hotkey-capture-1");
    root = createRoot(host);
  });
});

async function render(root: Root, ui: ReactNode) {
  await act(async () => root.render(ui));
}

async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

async function flushAnimationFrame() {
  await act(async () => {
    await new Promise<void>((resolve) => window.requestAnimationFrame(() => resolve()));
  });
}
