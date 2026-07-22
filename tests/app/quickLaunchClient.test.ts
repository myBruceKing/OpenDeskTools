import { describe, expect, it } from "vitest";
import { createQuickLaunchClient } from "../../src/app/quickLaunchClient";

describe("quick launch icon transport", () => {
  it("accepts Tauri Response ArrayBuffer payloads just like clipboard icons", async () => {
    const bytes = new Uint8Array([137, 80, 78, 71]).buffer;
    const client = createQuickLaunchClient(async (command) => {
      expect(command).toBe("get_quick_launch_icon");
      return bytes;
    });

    const icon = await client.getIcon("C:\\Tools\\Demo.exe");
    expect(icon.type).toBe("image/png");
    expect(await icon.arrayBuffer()).toEqual(bytes);
  });

  it("uses the explicit slot-swap command for radial menu reordering", async () => {
    const client = createQuickLaunchClient(async (command, args) => {
      expect(command).toBe("swap_quick_launch_apps");
      expect(args).toEqual({ input: { activePath: "C:\\Apps\\Ding.exe", overPath: "C:\\Apps\\Edge.exe" } });
      return {
        pinnedApps: [],
        discoveredApps: [],
        toolMenu: { layout: "wheel", keepOpenOnKeyRelease: false }
      };
    });

    await expect(client.swap("C:\\Apps\\Ding.exe", "C:\\Apps\\Edge.exe")).resolves.toMatchObject({
      toolMenu: { layout: "wheel" }
    });
  });
});
