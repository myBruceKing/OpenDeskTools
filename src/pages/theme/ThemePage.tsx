import {
  Color24Regular,
  Info20Regular,
  Rocket24Regular,
  Screenshot24Regular
} from "@fluentui/react-icons";
import { PageScaffold } from "../../components/layout/PageScaffold";
import { SettingsCard } from "../../components/layout/SettingsCard";
import { ListRowDescription, ListRowTitle } from "../../components/patterns/ListRow";
import { SectionTitle } from "../../components/patterns/Section";
import { SelectField } from "../../components/primitives/Field";
import { SegmentedControl } from "../../components/primitives/SelectionControl";
import { FieldRow, SwitchRow } from "../static/SettingsRows";
import {
  ANIMATION_SPEEDS,
  THEME_ACCENTS,
  type AnimationSpeed,
  type ThemeControllerState,
  type ThemeMode,
  type ThemePatch
} from "../../app/themeModel";
import styles from "../static/SettingsPages.module.css";

type ThemePageProps = {
  state: ThemeControllerState;
  onUpdate: (patch: ThemePatch) => Promise<void>;
};

const modeOptions = [
  { value: "system", label: "跟随系统" },
  { value: "light", label: "浅色" },
  { value: "dark", label: "深色" }
];

const animationLabels: Record<AnimationSpeed, string> = {
  slow: "舒缓",
  normal: "标准",
  fast: "快速"
};

export function ThemePage({ state, onUpdate }: ThemePageProps) {
  const ready = state.status === "ready" && state.current !== null;
  const current = state.current;

  const update = (patch: ThemePatch) => {
    void onUpdate(patch);
  };

  const description =
    state.status === "loading"
      ? "正在加载主题设置…"
      : state.status === "unavailable"
        ? "主题服务当前不可用；设置暂不可修改。"
        : state.saving
          ? "设置正在保存，并会同步到其他窗口。"
          : "设置会立即生效，并同步到其他窗口。";

  return (
    <PageScaffold title="主题" description={description}>
      {(state.error || state.warning) && (
        <div
          className={state.error ? styles.themeIssueError : styles.themeIssueWarning}
          role={state.error ? "alert" : "status"}
        >
          {state.error?.message ?? state.warning?.message}
        </div>
      )}
      <div className={styles.themeGrid}>
        <SettingsCard fill>
          <SectionTitle>主题设置</SectionTitle>
          <SegmentedControl
            label="主题模式"
            value={current?.mode ?? "unavailable"}
            options={modeOptions}
            onChange={(value) => update({ mode: value as ThemeMode })}
            disabled={!ready}
          />
          <div className={styles.swatches}>
            {THEME_ACCENTS.map((color) => (
              <button
                className={[styles.swatch, current?.accent === color ? styles.swatchActive : ""]
                  .filter(Boolean)
                  .join(" ")}
                style={{ backgroundColor: color }}
                type="button"
                aria-label={`使用主题色 ${color}`}
                aria-pressed={current?.accent === color}
                key={color}
                disabled={!ready}
                onClick={() => update({ accent: color })}
              />
            ))}
          </div>
          <SwitchRow
            title="减少透明效果"
            description="降低半透明和模糊层级"
            checked={current?.reduceTransparency ?? null}
            disabled={!ready}
            onChange={(reduceTransparency) => update({ reduceTransparency })}
          />
          <FieldRow label="动画速度">
            <SelectField
              value={current?.animationSpeed ?? ""}
              disabled={!ready}
              onChange={(event) => update({ animationSpeed: event.target.value as AnimationSpeed })}
            >
              {!current && <option value="">—</option>}
              {ANIMATION_SPEEDS.map((speed) => (
                <option value={speed} key={speed}>{animationLabels[speed]}</option>
              ))}
            </SelectField>
          </FieldRow>
        </SettingsCard>
        <SettingsCard fill>
          <div className={styles.featureTitle}>
            <Color24Regular className={styles.featureTitleIcon} aria-hidden="true" />
            <SectionTitle>主页面主题</SectionTitle>
          </div>
          <div className={styles.themePreviewWindow}>
            <span className={styles.themePreviewSidebar} />
            <span className={styles.themePreviewContent} />
            <span className={styles.themePreviewAccent} />
          </div>
        </SettingsCard>
        <SettingsCard fill>
          <div className={styles.featureTitle}>
            <Info20Regular className={styles.featureTitleIcon} aria-hidden="true" />
            <SectionTitle>简洁弹窗主题</SectionTitle>
          </div>
          <div className={styles.themePreviewToast}>
            <span className={styles.themePreviewDot} />
            <div>
              <strong>视觉示意</strong>
              <span>通知样式预览</span>
            </div>
          </div>
        </SettingsCard>
      </div>
      <SettingsCard>
        <SectionTitle>临时界面主题预览</SectionTitle>
        <div className={styles.themeSurfaceGrid}>
          <div className={styles.themeSurfaceCard}>
            <Screenshot24Regular className={styles.featureTitleIcon} aria-hidden="true" />
            <div>
              <ListRowTitle>截图遮罩</ListRowTitle>
              <ListRowDescription>选区边框、尺寸标签和遮罩色跟随主题。</ListRowDescription>
            </div>
          </div>
          <div className={styles.themeSurfaceCard}>
            <Rocket24Regular className={styles.featureTitleIcon} aria-hidden="true" />
            <div>
              <ListRowTitle>快速弹窗</ListRowTitle>
              <ListRowDescription>快速启动、剪贴板等轻量面板使用同一主题令牌。</ListRowDescription>
            </div>
          </div>
        </div>
      </SettingsCard>
    </PageScaffold>
  );
}
