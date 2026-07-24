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
      if (command === "update_hotkey_binding") {
        return {
          snapshot: backendSnapshot,
          systemHotkeyNotice: { binding: "Win+V", letter: "V", restartRequired: true }
        };
      }
      if (command === "update_hotkey_enabled") {
        return {
          snapshot: {
            ...backendSnapshot,
            revision: 5,
            actions: [{
              ...backendSnapshot.actions[0],
              configuredEnabled: false,
              runtimeState: "disabled",
              runtimeBackend: null,
              detail: "快捷键未启用。"
            }]
          },
          systemHotkeyNotice: null
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
    ).resolves.toEqual({
      snapshot,
      systemHotkeyNotice: { binding: "Win+V", letter: "V", restartRequired: true }
    });
    await expect(
      client.updateEnabled({
        actionId: "capture",
        expectedRevision: 4,
        enabled: false
      })
    ).resolves.toEqual({
      snapshot: {
        revision: 5,
        actions: [{
          ...snapshot.actions[0],
          configuredEnabled: false,
          runtimeState: "disabled",
          runtimeBackend: null,
          detail: "快捷键未启用。"
        }]
      },
      systemHotkeyNotice: null
    });

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
    expect(invokeFunction).toHaveBeenNthCalledWith(4, "update_hotkey_enabled", {
      patch: {
        actionId: "screenshot.capture",
        expectedRevision: 4,
        enabled: false
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

  it("maps runtime state to a truthful badge state", () => {
    expect(toHotkeyBadgeState("registered")).toBe("normal");
    expect(toHotkeyBadgeState("conflict")).toBe("conflict");
    expect(toHotkeyBadgeState("disabled")).toBe("disabled");
    expect(toHotkeyBadgeState("unavailable")).toBe("unavailable");
    expect(toHotkeyBadgeState("degraded")).toBe("unavailable");
  });
});
