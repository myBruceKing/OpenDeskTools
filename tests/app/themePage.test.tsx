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
    revision: 7
  },
  current: {
    mode: "dark",
    accent: "#008b83",
    animationSpeed: "fast",
    reduceTransparency: true,
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
        onUpdate={vi.fn()}
      />
    );

    expect(html).toContain("主题服务当前不可用");
    expect(html).toContain("主题服务不可用");
    expect(html).toContain("disabled");
    expect(html).toContain('data-state="unavailable"');
    expect(html).not.toContain('aria-checked="true"');
  });

  it("renders confirmed settings as enabled controlled values", () => {
    const html = renderToStaticMarkup(<ThemePage state={readyState} onUpdate={vi.fn()} />);

    expect(html).toContain('aria-label="使用主题色 #008b83"');
    expect(html).toContain('aria-pressed="true"');
    expect(html).toContain('role="switch" aria-checked="true"');
    expect(html).toContain('<option value="fast" selected="">快速</option>');
    expect(html).toContain('role="radio" aria-checked="true"');
  });
});
