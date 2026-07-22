import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn()
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));

import { generalClient } from "../../src/app/generalClient";
import {
  createGeneralViewModel,
  parseGeneralCommandError
} from "../../src/app/generalModel";

const backendSnapshot = {
  version: "0.1.0",
  autostartEnabled: true,
  startMinimized: false,
  closeToTray: true,
  crashDiagnosticsEnabled: false,
  dataDirectory: "C:\\Users\\me\\AppData\\Roaming\\com.opendesktools.app"
};

describe("generalModel", () => {
  it("maps the backend snapshot and falls back to null when absent", () => {
    expect(createGeneralViewModel(backendSnapshot)).toEqual({
      version: "0.1.0",
      autostartEnabled: true,
      startMinimized: false,
      closeToTray: true,
      crashDiagnosticsEnabled: false,
      dataDirectory: "C:\\Users\\me\\AppData\\Roaming\\com.opendesktools.app"
    });
    expect(createGeneralViewModel(null)).toEqual({
      version: null,
      autostartEnabled: null,
      startMinimized: null,
      closeToTray: null,
      crashDiagnosticsEnabled: null,
      dataDirectory: null
    });
  });

  it("extracts the command error message with a stable fallback", () => {
    expect(parseGeneralCommandError({ code: "autostart_update_failed", message: "开机自启设置未生效：拒绝访问" })).toBe(
      "开机自启设置未生效：拒绝访问"
    );
    expect(parseGeneralCommandError(new Error("boom"))).toBe("boom");
    expect(parseGeneralCommandError("weird")).toBe("设置未生效，请重试。");
  });
});

describe("generalClient", () => {
  beforeEach(() => {
    mocks.invoke.mockReset();
  });

  it("loads settings through the frozen command name", async () => {
    mocks.invoke.mockResolvedValueOnce(backendSnapshot);

    const viewModel = await generalClient.load();

    expect(mocks.invoke).toHaveBeenCalledWith("get_general_settings");
    expect(viewModel.autostartEnabled).toBe(true);
  });

  it("maps each toggle kind to its frozen command and passes the enabled flag", async () => {
    mocks.invoke.mockResolvedValue(backendSnapshot);

    await generalClient.setToggle("autostart", false);
    expect(mocks.invoke).toHaveBeenLastCalledWith("set_autostart_enabled", { enabled: false });

    await generalClient.setToggle("startMinimized", true);
    expect(mocks.invoke).toHaveBeenLastCalledWith("set_start_minimized", { enabled: true });

    await generalClient.setToggle("closeToTray", false);
    expect(mocks.invoke).toHaveBeenLastCalledWith("set_close_to_tray", { enabled: false });

    await generalClient.setToggle("crashDiagnostics", true);
    expect(mocks.invoke).toHaveBeenLastCalledWith("set_crash_diagnostics_enabled", { enabled: true });
  });

  it("opens the native picker then migrates through the frozen command", async () => {
    mocks.invoke.mockResolvedValueOnce({ dataDirectory: "D:\\OpenDeskTools", restartRequired: true });

    await expect(generalClient.selectAndMigrateDataDirectory()).resolves.toEqual({
      dataDirectory: "D:\\OpenDeskTools",
      restartRequired: true
    });
    expect(mocks.invoke).toHaveBeenCalledWith("select_and_migrate_data_directory");
  });
});
