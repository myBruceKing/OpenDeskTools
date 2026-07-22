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
});
