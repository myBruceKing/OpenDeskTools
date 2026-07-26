import { describe, expect, it, vi } from "vitest";
import { ControllerListeners } from "../../src/app/controllerListeners";

describe("ControllerListeners", () => {
  it("notifies subscribed listeners in insertion order", () => {
    const listeners = new ControllerListeners();
    const calls: string[] = [];

    listeners.subscribe(() => calls.push("first"));
    listeners.subscribe(() => calls.push("second"));
    listeners.notify();

    expect(calls).toEqual(["first", "second"]);
  });

  it("supports idempotent unsubscription without affecting other listeners", () => {
    const listeners = new ControllerListeners();
    const removed = vi.fn();
    const retained = vi.fn();
    const unsubscribe = listeners.subscribe(removed);
    listeners.subscribe(retained);

    unsubscribe();
    unsubscribe();
    listeners.notify();

    expect(removed).not.toHaveBeenCalled();
    expect(retained).toHaveBeenCalledOnce();
  });
});
