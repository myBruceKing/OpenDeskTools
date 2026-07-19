// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ImagePreview } from "../../src/components/patterns/ImagePreview";
import {
  useClipboardImagePreview,
  type LoadClipboardImage
} from "../../src/pages/clipboard/useClipboardImagePreview";

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

function Harness({ itemId, loadImage }: { itemId: string | null; loadImage: LoadClipboardImage }) {
  const preview = useClipboardImagePreview(itemId, loadImage);
  return (
    <>
      <ImagePreview
        state={preview.state}
        alt="测试图片"
        onLoad={preview.markLoaded}
        onError={preview.markDecodeError}
        onRetry={preview.retry}
      />
      <button type="button" data-action="release" onClick={preview.release}>释放</button>
    </>
  );
}

describe("useClipboardImagePreview", () => {
  let host: HTMLDivElement;
  let root: Root;
  let nextUrl: number;
  let createObjectUrl: ReturnType<typeof vi.fn>;
  let revokeObjectUrl: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
    nextUrl = 0;
    createObjectUrl = vi.fn(() => `blob:clipboard-${++nextUrl}`);
    revokeObjectUrl = vi.fn();
    Object.defineProperty(URL, "createObjectURL", { configurable: true, value: createObjectUrl });
    Object.defineProperty(URL, "revokeObjectURL", { configurable: true, value: revokeObjectUrl });
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    document.body.replaceChildren();
    vi.restoreAllMocks();
  });

  async function render(itemId: string | null, loadImage: LoadClipboardImage) {
    await act(async () => {
      root.render(<Harness itemId={itemId} loadImage={loadImage} />);
    });
  }

  async function settle() {
    await act(async () => {
      for (let index = 0; index < 6; index += 1) {
        await Promise.resolve();
      }
    });
  }

  it("loads only the latest A to B selection and revokes its URL exactly once", async () => {
    const first = deferred<Blob>();
    const second = deferred<Blob>();
    const loadImage = vi.fn((id: string) => id === "A" ? first.promise : second.promise);

    await render("A", loadImage);
    expect(document.body.textContent).toContain("正在加载图片预览");
    await render("B", loadImage);
    expect(document.querySelector("img")).toBeNull();

    first.resolve(new Blob([new Uint8Array([1])], { type: "image/png" }));
    await settle();
    expect(createObjectUrl).not.toHaveBeenCalled();

    second.resolve(new Blob([new Uint8Array([2])], { type: "image/png" }));
    await settle();
    expect(createObjectUrl).toHaveBeenCalledOnce();
    expect(document.querySelector("[data-image-preview-state='decoding']")).toBeTruthy();

    const image = document.querySelector<HTMLImageElement>("img")!;
    await act(async () => image.dispatchEvent(new Event("load")));
    expect(document.querySelector("[data-image-preview-state='ready']")).toBeTruthy();

    await render(null, loadImage);
    expect(revokeObjectUrl).toHaveBeenCalledTimes(1);
    expect(revokeObjectUrl).toHaveBeenCalledWith("blob:clipboard-1");
    expect(document.querySelector("[data-image-preview-state='idle']")).toBeTruthy();
    expect(loadImage).toHaveBeenCalledTimes(2);
  });

  it("hides and revokes a visible A before a pending B can replace it", async () => {
    const second = deferred<Blob>();
    const loadImage = vi.fn((id: string) => id === "A"
      ? Promise.resolve(new Blob([new Uint8Array([1])]))
      : second.promise);

    await render("A", loadImage);
    await settle();
    const firstImage = document.querySelector<HTMLImageElement>("img")!;
    await act(async () => firstImage.dispatchEvent(new Event("load")));
    expect(document.querySelector("[data-image-preview-state='ready']")).toBeTruthy();

    await render("B", loadImage);
    expect(document.querySelector("img")).toBeNull();
    expect(revokeObjectUrl.mock.calls).toEqual([["blob:clipboard-1"]]);

    second.resolve(new Blob([new Uint8Array([2])]));
    await settle();
    expect(document.querySelector("[data-image-preview-state='decoding']")).toBeTruthy();

    await act(async () => root.unmount());
    expect(revokeObjectUrl.mock.calls).toEqual([
      ["blob:clipboard-1"],
      ["blob:clipboard-2"]
    ]);
    root = createRoot(host);
  });

  it("releases pending work immediately, ignores the late response, and retries on demand", async () => {
    const first = deferred<Blob>();
    const second = deferred<Blob>();
    const loadImage = vi.fn()
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise);

    await render("A", loadImage);
    const release = document.querySelector<HTMLButtonElement>("[data-action='release']")!;
    await act(async () => release.click());
    expect(document.body.textContent).toContain("图片预览已释放");

    first.resolve(new Blob([new Uint8Array([1])]));
    await settle();
    expect(createObjectUrl).not.toHaveBeenCalled();

    const retry = Array.from(document.querySelectorAll<HTMLButtonElement>("button"))
      .find((button) => button.textContent === "重试")!;
    await act(async () => retry.click());
    second.resolve(new Blob([new Uint8Array([2])]));
    await settle();
    expect(createObjectUrl).toHaveBeenCalledOnce();
  });

  it("revokes a decoded URL once on decode failure and allows a retry", async () => {
    const loadImage = vi.fn(async () => new Blob([new Uint8Array([1])]));
    await render("A", loadImage);
    await settle();

    const image = document.querySelector<HTMLImageElement>("img")!;
    await act(async () => image.dispatchEvent(new Event("error")));
    expect(document.body.textContent).toContain("图片无法解码");
    expect(revokeObjectUrl).toHaveBeenCalledTimes(1);

    const retry = Array.from(document.querySelectorAll<HTMLButtonElement>("button"))
      .find((button) => button.textContent === "重试")!;
    await act(async () => retry.click());
    await settle();
    expect(loadImage).toHaveBeenCalledTimes(2);
    expect(createObjectUrl).toHaveBeenCalledTimes(2);

    await act(async () => root.unmount());
    expect(revokeObjectUrl.mock.calls).toEqual([
      ["blob:clipboard-1"],
      ["blob:clipboard-2"]
    ]);
    root = createRoot(host);
  });

  it.each([
    ["oversized", { code: "clipboard_image_too_large" }, "oversized", "图片过大"],
    ["unavailable", { code: "clipboard_image_unavailable" }, "unavailable", "图片内容不可用"],
    ["retryable", { code: "clipboard_history_unavailable" }, "error", "图片加载失败"]
  ] as const)("renders the %s state without creating a URL", async (_label, error, state, copy) => {
    const loadImage = vi.fn(async () => Promise.reject(error));
    await render("A", loadImage);
    await settle();

    expect(document.querySelector(`[data-image-preview-state='${state}']`)).toBeTruthy();
    expect(document.body.textContent).toContain(copy);
    expect(createObjectUrl).not.toHaveBeenCalled();
  });

  it("treats an empty blob as unavailable", async () => {
    await render("A", async () => new Blob());
    await settle();
    expect(document.querySelector("[data-image-preview-state='unavailable']")).toBeTruthy();
    expect(document.body.textContent).toContain("图片内容为空");
  });
});
