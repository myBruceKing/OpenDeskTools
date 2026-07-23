// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { RangeNumberField } from "../../src/components/primitives/Field";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

describe("RangeNumberField", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    document.body.replaceChildren();
  });

  it("updates continuously from the slider and clamps direct numeric input", async () => {
    const onChange = vi.fn();
    await act(async () => {
      root.render(
        <RangeNumberField
          label="背景遮罩"
          value={24}
          min={0}
          max={100}
          unit="%"
          onChange={onChange}
        />
      );
    });

    const slider = host.querySelector('[aria-label="背景遮罩滑杆"]') as HTMLInputElement;
    const number = host.querySelector('[aria-label="背景遮罩数值"]') as HTMLInputElement;

    await act(async () => {
      setInputValue(slider, "100");
      slider.dispatchEvent(new Event("change", { bubbles: true }));
      slider.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(onChange).not.toHaveBeenCalled();

    await act(async () => {
      slider.dispatchEvent(new Event("pointerup", { bubbles: true }));
    });
    expect(onChange).toHaveBeenCalledWith(100);

    await act(async () => {
      number.focus();
      setInputValue(number, "132");
      number.dispatchEvent(new Event("input", { bubbles: true }));
      number.dispatchEvent(new Event("change", { bubbles: true }));
      number.blur();
    });
    expect(onChange).toHaveBeenLastCalledWith(100);
    expect(number.value).toBe("100");
  });
});

function setInputValue(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
  setter?.call(input, value);
}
