import { existsSync, readdirSync, readFileSync } from "node:fs";
import { extname, join, resolve } from "node:path";
import { describe, expect, it } from "vitest";

const projectRoot = resolve(import.meta.dirname, "../..");
const sourceRoot = join(projectRoot, "src");

function sourceFiles(directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      return sourceFiles(path);
    }
    return [".ts", ".tsx", ".css"].includes(extname(entry.name)) ? [path] : [];
  });
}

describe("production source boundary", () => {
  it("keeps visual fixtures outside src and out of production imports", () => {
    expect(existsSync(join(sourceRoot, "app", "previewData.ts"))).toBe(false);

    const productionSource = sourceFiles(sourceRoot)
      .map((path) => readFileSync(path, "utf8"))
      .join("\n");

    expect(productionSource).not.toMatch(/from\s+["'][^"']*visual-fixtures/i);
    expect(productionSource).not.toMatch(/from\s+["'][^"']*previewData/i);
  });

  it("does not contain known prototype-only runtime values", () => {
    const productionSource = sourceFiles(sourceRoot)
      .map((path) => readFileSync(path, "utf8"))
      .join("\n");

    for (const marker of [
      "notepad.exe",
      "2024-05-21",
      "与系统快捷键冲突",
      "C:\\\\Users\\\\",
      "%APPDATA%",
      "Program Files",
      "Snipaste",
      "todayTriggers: 128",
      "weekTriggers: 1248",
      "monthTriggers: 8653",
      "savedSecondsThisMonth: 547200"
    ]) {
      expect(productionSource, marker).not.toContain(marker);
    }

    expect(productionSource).not.toMatch(/snipaste/i);
  });
});
