import { describe, expect, it, vi } from "vitest";
import { createHotkeyClient } from "../../src/app/hotkeyClient";
import { toHotkeyBadgeState } from "../../src/app/hotkeyModel";

const backendSnapshot = {
  revision: 4,
  actions: [
    {
      actionId: "screenshot.capture",
      binding: "F1",
      configuredEnabled: true,
      classification: "ordinary",
      runtimeState: "registered",
      runtimeBackend: "standard",
      detail: null,
      actionAvailable: true,
      forceOverrideSystem: false
    }
  ]
};

const snapshot = {
  ...backendSnapshot,
  actions: [{ ...backendSnapshot.actions[0], actionId: "capture" }]
};

describe("hotkeyClient", () => {
  it("maps the frozen get/classify/update commands and arguments", async () => {
    const invokeFunction = vi.fn(async (command: string) => {
      if (command === "classify_hotkey_binding") {
        return {
          binding: "Win+V",
          normalizedBinding: "Win+V",
          classification: "system_reserved",
          message: "系统快捷键",
          forceOverrideAllowed: true
        };
      }
      return backendSnapshot;
    });
    const client = createHotkeyClient({ invokeFunction });

    await expect(client.getSnapshot()).resolves.toEqual(snapshot);
    await expect(client.classify("Win+V")).resolves.toEqual({
      binding: "Win+V",
      normalizedBinding: "Win+V",
      classification: "system_reserved",
      message: "系统快捷键",
      forceOverrideAllowed: true
    });
    await expect(
      client.update({
        actionId: "capture",
        expectedRevision: 4,
        binding: "Win+V",
        forceOverrideSystem: true
      })
    ).resolves.toEqual(snapshot);

    expect(invokeFunction).toHaveBeenNthCalledWith(1, "get_hotkey_snapshot");
    expect(invokeFunction).toHaveBeenNthCalledWith(2, "classify_hotkey_binding", {
      binding: "Win+V"
    });
    expect(invokeFunction).toHaveBeenNthCalledWith(3, "update_hotkey_binding", {
      patch: {
        actionId: "screenshot.capture",
        expectedRevision: 4,
        binding: "Win+V",
        forceOverrideSystem: true
      }
    });
  });

  it("rejects unknown actions, states, and classifications instead of inventing UI state", async () => {
    const invalidActionClient = createHotkeyClient({
      invokeFunction: async () => ({
        ...backendSnapshot,
        actions: [{ ...backendSnapshot.actions[0], actionId: "unknown-action" }]
      })
    });
    await expect(invalidActionClient.getSnapshot()).rejects.toThrow("actionId");

    const invalidRuntimeClient = createHotkeyClient({
      invokeFunction: async () => ({
        ...backendSnapshot,
        actions: [{ ...backendSnapshot.actions[0], runtimeState: "normal" }]
      })
    });
    await expect(invalidRuntimeClient.getSnapshot()).rejects.toThrow("runtimeState");

    const invalidBackendClient = createHotkeyClient({
      invokeFunction: async () => ({
        ...backendSnapshot,
        actions: [{ ...backendSnapshot.actions[0], runtimeBackend: "policy_only" }]
      })
    });
    await expect(invalidBackendClient.getSnapshot()).rejects.toThrow("runtimeBackend");

    const invalidClassificationClient = createHotkeyClient({
      invokeFunction: async () => ({
        binding: "Ctrl+K",
        normalizedBinding: "Ctrl+K",
        classification: "probably-safe",
        message: "unknown",
        forceOverrideAllowed: false
      })
    });
    await expect(invalidClassificationClient.classify("Ctrl+K")).rejects.toThrow(
      "classification"
    );
  });

  it("maps only registered runtime state to the normal badge", () => {
    expect(toHotkeyBadgeState("registered")).toBe("normal");
    expect(toHotkeyBadgeState("conflict")).toBe("conflict");
    expect(toHotkeyBadgeState("disabled")).toBe("unavailable");
    expect(toHotkeyBadgeState("unavailable")).toBe("unavailable");
    expect(toHotkeyBadgeState("degraded")).toBe("unavailable");
  });
});
