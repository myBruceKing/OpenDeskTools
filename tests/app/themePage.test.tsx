import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { ThemePage } from "../../src/pages/theme/ThemePage";
import type { ThemeControllerState } from "../../src/app/themeModel";

const readyState: ThemeControllerState = {
  status: "ready",
  confirmed: {
    mode: "dark",
    accent: "#008b83",
    animationSpeed: "fast",
    reduceTransparency: true,
    background: null,
    backgroundFit: "cover",
    backgroundDim: 24,
    backgroundBlur: 6,
    panelOpacity: 86,
    revision: 7
  },
  current: {
    mode: "dark",
    accent: "#008b83",
    animationSpeed: "fast",
    reduceTransparency: true,
    background: null,
    backgroundFit: "cover",
    backgroundDim: 24,
    backgroundBlur: 6,
    panelOpacity: 86,
    revision: 7
  },
  saving: false,
  error: null,
  warning: null
};

describe("ThemePage", () => {
  it("renders unavailable controls disabled without invented values", () => {
    const html = renderToStaticMarkup(
      <ThemePage
        state={{
          status: "unavailable",
          confirmed: null,
          current: null,
          saving: false,
          error: {
            code: "theme_unavailable",
            message: "主题服务不可用",
            field: null,
            retryable: true,
            applied: false
          },
          warning: null
        }}
        backgroundImageState={{ status: "idle", url: null }}
        onUpdate={vi.fn()}
        onSelectBackground={vi.fn()}
        onRemoveBackground={vi.fn()}
      />
    );

    expect(html).toContain("主题服务当前不可用");
    expect(html).toContain("主题服务不可用");
    expect(html).toContain("disabled");
    expect(html).toContain('data-state="unavailable"');
    expect(html).not.toContain('aria-checked="true"');
  });

  it("renders confirmed settings as enabled controlled values", () => {
    const html = renderToStaticMarkup(
      <ThemePage
        state={readyState}
        backgroundImageState={{ status: "idle", url: null }}
        onUpdate={vi.fn()}
        onSelectBackground={vi.fn()}
        onRemoveBackground={vi.fn()}
      />
    );

    expect(html).toContain('aria-label="使用主题色 #008b83"');
    expect(html).toContain('aria-pressed="true"');
    expect(html).toContain('role="switch" aria-checked="true"');
    expect(html).toContain('<option value="fast" selected="">快速</option>');
    expect(html).toContain('role="radio" aria-checked="true"');
    expect(html).toContain("图片皮肤");
    expect(html).toContain("色盘");
    expect(html).toContain('type="color"');
    expect(html).toContain("选择图片");
    expect(html).toContain("恢复默认背景");
    expect(html).toContain("选择图片后立即应用");
    expect(html).toContain("skinHeaderDescription");
    expect(html).toContain("未选择背景图片");
    expect(html.match(/type="range"/g)).toHaveLength(3);
    expect(html.match(/type="number"/g)).toHaveLength(3);
    expect(html).toContain('aria-label="背景遮罩滑杆" value="24"');
    expect(html).toContain('aria-label="背景遮罩数值" value="24"');
    expect(html).toContain('max="100"');
    expect(html).toContain('data-disabled="true"');
    expect(html).not.toContain("当前使用默认背景");
    expect(html).not.toContain("移动或删除原文件不会影响皮肤");
    expect(html).not.toContain("选择 PNG、JPEG 或 WebP");
    expect(html).not.toContain("主页面主题");
    expect(html).not.toContain("简洁弹窗主题");
    expect(html).not.toContain("临时界面主题预览");
    expect(html).not.toContain("截图遮罩");
    expect(html).not.toContain("快速弹窗");
  });

  it("renders managed image metadata and material controls without exposing a local path", () => {
    const background = {
      id: "c".repeat(64),
      fileName: "山林.webp",
      byteSize: 8192,
      width: 1920,
      height: 1080
    };
    const state: ThemeControllerState = {
      ...readyState,
      confirmed: { ...readyState.confirmed!, background, reduceTransparency: false },
      current: { ...readyState.current!, background, reduceTransparency: false }
    };
    const html = renderToStaticMarkup(
      <ThemePage
        state={state}
        backgroundImageState={{ status: "ready", url: "blob:managed-background" }}
        onUpdate={vi.fn()}
        onSelectBackground={vi.fn()}
        onRemoveBackground={vi.fn()}
      />
    );

    expect(html).toContain("山林.webp");
    expect(html).toContain("1920 × 1080");
    expect(html).toContain("更换图片");
    expect(html).toContain("恢复默认背景");
    expect(html).toContain('src="blob:managed-background"');
    expect(html).toContain('data-disabled="false"');
    expect(html).toContain('aria-label="背景遮罩滑杆" value="24"');
    expect(html).toContain('aria-label="面板透明度数值" value="86"');
    expect(html).not.toMatch(/[A-Z]:\\/);
  });
});
