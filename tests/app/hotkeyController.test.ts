import { describe, expect, it, vi } from "vitest";
import type { HotkeyClient } from "../../src/app/hotkeyClient";
import {
  HotkeyController,
  appendHotkeyToken,
  canSaveHotkeyEditor
} from "../../src/app/hotkeyController";
import type {
  HotkeyClassification,
  HotkeySnapshot,
  HotkeyUpdatePatch
} from "../../src/app/hotkeyModel";

function snapshot(
  revision = 1,
  overrides: Partial<HotkeySnapshot["actions"][number]> = {}
): HotkeySnapshot {
  return {
    revision,
    actions: [
      {
        actionId: "capture",
        binding: "F1",
        configuredEnabled: true,
        classification: "ordinary",
        runtimeState: "registered",
        runtimeBackend: "standard",
        detail: null,
        actionAvailable: true,
        forceOverrideSystem: false,
        ...overrides
      }
    ]
  };
}

function classified(
  classification: HotkeyClassification["classification"],
  message = ""
): HotkeyClassification {
  return {
    binding: "F1",
    normalizedBinding: "F1",
    classification,
    message,
    forceOverrideAllowed: classification === "system_reserved"
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

async function flush() {
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
}

function client(overrides: Partial<HotkeyClient> = {}): HotkeyClient {
  return {
    getSnapshot: async () => snapshot(),
    classify: async () => classified("ordinary"),
    update: async () => snapshot(2),
    ...overrides
  };
}

describe("HotkeyController", () => {
  it("replaces the saved binding on first capture and appends only later sequence tokens", async () => {
    expect(appendHotkeyToken("F1", "Shift+Win+S")).toBe("F1 Shift+Win+S");
    expect(appendHotkeyToken(" F1  ", "  ")).toBe("F1");

    const classify = vi.fn<HotkeyClient["classify"]>(async () => classified("ordinary"));
    const controller = new HotkeyController(client({ classify }));
    controller.start();
    await flush();
    controller.openEditor("capture");
    await flush();
    controller.appendBindingToken("Shift+Win+S");

    expect(controller.getSnapshot().editor?.binding).toBe("Shift+Win+S");
    expect(classify).toHaveBeenLastCalledWith("Shift+Win+S");

    controller.appendBindingToken("F2");
    expect(controller.getSnapshot().editor?.binding).toBe("Shift+Win+S F2");
    expect(classify).toHaveBeenLastCalledWith("Shift+Win+S F2");
  });

  it("loads a dedicated hotkey snapshot and reports unavailable truthfully", async () => {
    const controller = new HotkeyController(client());
    controller.start();
    await flush();
    expect(controller.getSnapshot()).toMatchObject({ status: "ready", snapshot: snapshot() });

    const unavailable = new HotkeyController(
      client({
        getSnapshot: async () => Promise.reject({ code: "hotkey_unavailable", message: "不可用" })
      })
    );
    unavailable.start();
    await flush();
    expect(unavailable.getSnapshot()).toMatchObject({
      status: "unavailable",
      snapshot: null,
      error: { code: "hotkey_unavailable", message: "不可用" }
    });
  });

  it("ignores a stale classification response after the binding changes", async () => {
    const first = deferred<HotkeyClassification>();
    const second = deferred<HotkeyClassification>();
    const classify = vi
      .fn<HotkeyClient["classify"]>()
      .mockImplementationOnce(() => first.promise)
      .mockImplementationOnce(() => second.promise);
    const controller = new HotkeyController(client({ classify }));
    controller.start();
    await flush();

    controller.openEditor("capture");
    controller.setBinding("Ctrl+K");
    second.resolve(classified("ordinary"));
    await flush();
    first.resolve(classified("blocked", "旧响应"));
    await flush();

    expect(classify).toHaveBeenNthCalledWith(1, "F1");
    expect(classify).toHaveBeenNthCalledWith(2, "Ctrl+K");
    expect(controller.getSnapshot().editor).toMatchObject({
      binding: "Ctrl+K",
      classificationStatus: "ready",
      classification: { classification: "ordinary" }
    });
  });

  it.each([
    ["ordinary", false, true],
    ["system_reserved", false, false],
    ["system_reserved", true, true],
    ["blocked", true, false],
    ["unsupported_sequence", true, false]
  ] as const)(
    "applies the save gate for %s with force=%s",
    async (classification, forceOverrideSystem, expected) => {
      const controller = new HotkeyController(
        client({ classify: async () => classified(classification) })
      );
      controller.start();
      await flush();
      controller.openEditor("capture");
      await flush();
      controller.setForceOverrideSystem(forceOverrideSystem);

      expect(canSaveHotkeyEditor(controller.getSnapshot())).toBe(expected);
    }
  );

  it("restores saved system authorization and still allows unavailable actions to be configured", async () => {
    const controller = new HotkeyController(
      client({
        getSnapshot: async () =>
          snapshot(3, { actionAvailable: false, forceOverrideSystem: true, binding: "Win+V" }),
        classify: async () => classified("system_reserved", "系统保留")
      })
    );
    controller.start();
    await flush();
    controller.openEditor("capture");
    await flush();

    expect(controller.getSnapshot().editor).toMatchObject({
      actionAvailable: false,
      forceOverrideSystem: true
    });
    expect(canSaveHotkeyEditor(controller.getSnapshot())).toBe(true);
  });

  it("does not update the list optimistically and adopts only the returned backend snapshot", async () => {
    const request = deferred<HotkeySnapshot>();
    const update = vi.fn<(patch: HotkeyUpdatePatch) => Promise<HotkeySnapshot>>(
      () => request.promise
    );
    const controller = new HotkeyController(client({ update }));
    controller.start();
    await flush();
    controller.openEditor("capture");
    await flush();
    controller.setBinding("Ctrl+K");
    await flush();

    const saving = controller.save();
    expect(controller.getSnapshot()).toMatchObject({ snapshot: snapshot(), editor: { saving: true } });
    expect(update).toHaveBeenCalledWith({
      actionId: "capture",
      expectedRevision: 1,
      binding: "Ctrl+K",
      forceOverrideSystem: false
    });

    request.resolve(snapshot(2, { binding: "Ctrl+K", runtimeState: "registered" }));
    await saving;
    expect(controller.getSnapshot()).toMatchObject({
      snapshot: snapshot(2, { binding: "Ctrl+K", runtimeState: "registered" }),
      editor: null
    });
  });

  it("keeps the editor open when the returned snapshot did not persist the requested binding", async () => {
    const controller = new HotkeyController(
      client({ update: async () => snapshot(2, { binding: "F2" }) })
    );
    controller.start();
    await flush();
    controller.openEditor("capture");
    await flush();
    controller.setBinding("Win+V");
    await flush();
    controller.setForceOverrideSystem(true);

    await controller.save();

    expect(controller.getSnapshot()).toMatchObject({
      snapshot: snapshot(2, { binding: "F2" }),
      editor: {
        binding: "Win+V",
        saving: false,
        error: {
          code: "hotkey_update_not_applied",
          message: "保存未生效；快捷键服务当前仍返回 F2，请重试。"
        }
      }
    });
  });

  it("keeps the editor open when Win+V was returned without the requested force authorization", async () => {
    const update = vi.fn<HotkeyClient["update"]>(async () => snapshot(2, {
      binding: "Win+V",
      classification: "system_reserved",
      runtimeState: "unavailable",
      forceOverrideSystem: false
    }));
    const controller = new HotkeyController(
      client({
        classify: async () => classified("system_reserved", "系统保留"),
        update
      })
    );
    controller.start();
    await flush();
    controller.openEditor("capture");
    await flush();
    controller.setBinding("Win+V");
    await flush();
    controller.setForceOverrideSystem(true);

    await controller.save();

    expect(update).toHaveBeenCalledWith({
      actionId: "capture",
      expectedRevision: 1,
      binding: "Win+V",
      forceOverrideSystem: true
    });
    expect(controller.getSnapshot().editor).toMatchObject({
      binding: "Win+V",
      forceOverrideSystem: true,
      saving: false,
      error: {
        code: "hotkey_update_not_applied",
        message: "保存未生效；快捷键服务未确认 Win+V 的强制覆盖授权，请重试。"
      }
    });
  });

  it("does not accept forced Win+V as active without a confirmed runtime backend", async () => {
    const controller = new HotkeyController(
      client({
        classify: async () => classified("system_reserved", "系统保留"),
        update: async () => snapshot(2, {
          binding: "Win+V",
          classification: "system_reserved",
          runtimeState: "registered",
          runtimeBackend: null,
          forceOverrideSystem: true
        })
      })
    );
    controller.start();
    await flush();
    controller.openEditor("capture");
    await flush();
    controller.setBinding("Win+V");
    await flush();
    controller.setForceOverrideSystem(true);

    await controller.save();

    expect(controller.getSnapshot().editor).toMatchObject({
      binding: "Win+V",
      forceOverrideSystem: true,
      saving: false,
      error: {
        code: "hotkey_saved_not_active",
        message: "配置已保存为 Win+V，但快捷键服务未确认实际运行后端，请重试或重启应用后检查状态。"
      }
    });
  });

  it.each(["standard", "low_level_hook"] as const)(
    "closes after forced Win+V is registered on the %s runtime backend",
    async (runtimeBackend) => {
      const controller = new HotkeyController(
        client({
          classify: async () => classified("system_reserved", "系统保留"),
          update: async () => snapshot(2, {
            binding: "Win+V",
            classification: "system_reserved",
            runtimeState: "registered",
            runtimeBackend,
            forceOverrideSystem: true
          })
        })
      );
      controller.start();
      await flush();
      controller.openEditor("capture");
      await flush();
      controller.setBinding("Win+V");
      await flush();
      controller.setForceOverrideSystem(true);

      await controller.save();

      expect(controller.getSnapshot()).toMatchObject({
        snapshot: snapshot(2, {
          binding: "Win+V",
          classification: "system_reserved",
          runtimeState: "registered",
          runtimeBackend,
          forceOverrideSystem: true
        }),
        editor: null
      });
    }
  );

  it("keeps the confirmed binding visible when it was saved but registration is not active", async () => {
    const controller = new HotkeyController(
      client({
        update: async () => snapshot(2, {
          binding: "Ctrl+K",
          runtimeState: "conflict",
          detail: "系统拒绝注册。"
        })
      })
    );
    controller.start();
    await flush();
    controller.openEditor("capture");
    await flush();
    controller.setBinding("Ctrl+K");
    await flush();

    await controller.save();

    expect(controller.getSnapshot()).toMatchObject({
      snapshot: snapshot(2, {
        binding: "Ctrl+K",
        runtimeState: "conflict",
        detail: "系统拒绝注册。"
      }),
      editor: {
        binding: "Ctrl+K",
        saving: false,
        error: {
          code: "hotkey_saved_not_active",
          message: "配置已保存为 Ctrl+K，但当前未生效：系统拒绝注册。"
        }
      }
    });
  });

  it("keeps the confirmed snapshot and shows the backend error when saving fails", async () => {
    const controller = new HotkeyController(
      client({
        update: async () => Promise.reject({ code: "hotkey_revision_conflict", message: "配置已变化" })
      })
    );
    controller.start();
    await flush();
    controller.openEditor("capture");
    await flush();
    controller.setBinding("Ctrl+K");
    await flush();

    await controller.save();

    expect(controller.getSnapshot()).toMatchObject({
      snapshot: snapshot(),
      editor: {
        binding: "Ctrl+K",
        saving: false,
        error: { code: "hotkey_revision_conflict", message: "配置已变化" }
      }
    });
  });

  it("refreshes the confirmed revision after a revision conflict so retry can progress", async () => {
    const getSnapshot = vi
      .fn<HotkeyClient["getSnapshot"]>()
      .mockResolvedValueOnce(snapshot(1))
      .mockResolvedValueOnce(snapshot(7, { binding: "F2" }));
    const controller = new HotkeyController(
      client({
        getSnapshot,
        update: async () =>
          Promise.reject({ code: "hotkey_revision_conflict", message: "配置已变化" })
      })
    );
    controller.start();
    await flush();
    controller.openEditor("capture");
    await flush();
    controller.setBinding("Ctrl+K");
    await flush();

    await controller.save();

    expect(getSnapshot).toHaveBeenCalledTimes(2);
    expect(controller.getSnapshot()).toMatchObject({
      snapshot: snapshot(7, { binding: "F2" }),
      editor: {
        binding: "Ctrl+K",
        saving: false,
        error: { code: "hotkey_revision_conflict" }
      }
    });
  });

  it("does not classify or save after stop", async () => {
    const classify = vi.fn<HotkeyClient["classify"]>();
    const update = vi.fn<HotkeyClient["update"]>();
    const controller = new HotkeyController(client({ classify, update }));
    controller.start();
    await flush();
    controller.stop();

    controller.openEditor("capture");
    await controller.save();

    expect(classify).not.toHaveBeenCalled();
    expect(update).not.toHaveBeenCalled();
  });
});
