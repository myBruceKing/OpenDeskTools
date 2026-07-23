import { useRef } from "react";
import {
  ArrowReset24Regular,
  Color24Regular,
  ImageAdd24Regular
} from "@fluentui/react-icons";
import { PageScaffold } from "../../components/layout/PageScaffold";
import { SettingsCard } from "../../components/layout/SettingsCard";
import { ListRowDescription, ListRowTitle } from "../../components/patterns/ListRow";
import { SectionTitle } from "../../components/patterns/Section";
import { RangeNumberField, SelectField } from "../../components/primitives/Field";
import { Button } from "../../components/primitives/Button";
import { SegmentedControl } from "../../components/primitives/SelectionControl";
import { FieldRow, SwitchRow } from "../static/SettingsRows";
import {
  ANIMATION_SPEEDS,
  BACKGROUND_FITS,
  THEME_ACCENTS,
  type AnimationSpeed,
  type BackgroundFit,
  type ThemeControllerState,
  type ThemeMode,
  type ThemePatch
} from "../../app/themeModel";
import type { ThemeBackgroundImageState } from "../../app/themeRuntime";
import styles from "../static/SettingsPages.module.css";

type ThemePageProps = {
  state: ThemeControllerState;
  backgroundImageState: ThemeBackgroundImageState;
  onUpdate: (patch: ThemePatch) => Promise<void>;
  onSelectBackground: () => Promise<void>;
  onRemoveBackground: () => Promise<void>;
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

export function ThemePage({
  state,
  backgroundImageState,
  onUpdate,
  onSelectBackground,
  onRemoveBackground
}: ThemePageProps) {
  const ready = state.status === "ready" && state.current !== null;
  const current = state.current;
  const colorInputRef = useRef<HTMLInputElement>(null);
  const customAccentSelected = Boolean(
    current && !THEME_ACCENTS.some((color) => color === current.accent)
  );
  const skinIsDefault = !current
    || (
      current.background === null
      && current.backgroundFit === "cover"
      && current.backgroundDim === 24
      && current.backgroundBlur === 6
      && current.panelOpacity === 86
    );
  const hasBackground = Boolean(current?.background);
  const skinControlsDisabled = !ready || state.saving || !hasBackground;

  const update = (patch: ThemePatch) => {
    void onUpdate(patch);
  };

  const openColorPicker = () => {
    const input = colorInputRef.current;
    if (!input) {
      return;
    }
    if (typeof input.showPicker === "function") {
      input.showPicker();
    } else {
      input.click();
    }
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
        <SettingsCard>
          <SectionTitle>主题设置</SectionTitle>
          <div className={styles.themeSettingsGrid}>
            <div className={styles.themeSettingsGroup}>
              <SegmentedControl
                label="主题模式"
                value={current?.mode ?? "unavailable"}
                options={modeOptions}
                onChange={(value) => update({ mode: value as ThemeMode })}
                disabled={!ready}
              />
              <span className={styles.themeControlLabel}>主题色</span>
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
                <Button
                  className={styles.colorPickerButton}
                  variant={customAccentSelected ? "primary" : "outline"}
                  size="compact"
                  icon={<Color24Regular aria-hidden="true" />}
                  aria-pressed={customAccentSelected}
                  disabled={!ready || state.saving}
                  onClick={openColorPicker}
                >
                  色盘
                </Button>
                <input
                  ref={colorInputRef}
                  className={styles.nativeColorInput}
                  type="color"
                  tabIndex={-1}
                  aria-hidden="true"
                  disabled={!ready || state.saving}
                  value={current?.accent ?? THEME_ACCENTS[0]}
                  onChange={(event) => update({ accent: event.target.value.toLowerCase() })}
                />
              </div>
            </div>
            <div className={styles.themeSettingsGroup}>
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
            </div>
          </div>
        </SettingsCard>
        <SettingsCard>
          <div className={styles.skinHeader}>
            <SectionTitle>图片皮肤</SectionTitle>
            <div className={styles.skinActions}>
              <Button
                variant={current?.background ? "outline" : "primary"}
                icon={<ImageAdd24Regular aria-hidden="true" />}
                disabled={!ready || state.saving}
                onClick={() => void onSelectBackground()}
              >
                {current?.background ? "更换图片" : "选择图片"}
              </Button>
              <Button
                variant="outline"
                icon={<ArrowReset24Regular aria-hidden="true" />}
                disabled={!ready || state.saving || skinIsDefault}
                onClick={() => void onRemoveBackground()}
              >
                恢复默认背景
              </Button>
            </div>
            <ListRowDescription className={styles.skinHeaderDescription}>
              选择图片后立即应用；恢复默认会移除背景并重置下方参数。
            </ListRowDescription>
          </div>
          <div className={styles.skinEditor}>
            {current?.background && (
              <div className={styles.skinPreview} data-state={backgroundImageState.status}>
                {backgroundImageState.url ? (
                  <img
                    src={backgroundImageState.url}
                    alt=""
                    style={{ objectFit: current.backgroundFit }}
                  />
                ) : (
                  <span>
                    {backgroundImageState.status === "error"
                      ? "背景资源暂时无法读取"
                      : "正在加载背景…"}
                  </span>
                )}
              </div>
            )}
            <div className={styles.skinSettings}>
              <div className={styles.skinAssetMeta}>
                {current?.background ? (
                  <>
                  <ListRowTitle>{current.background.fileName}</ListRowTitle>
                  <ListRowDescription>
                    {`${current.background.width} × ${current.background.height} · ${Math.max(1, Math.round(current.background.byteSize / 1024))} KB`}
                  </ListRowDescription>
                  </>
                ) : (
                  <ListRowTitle>未选择背景图片</ListRowTitle>
                )}
              </div>
              <div className={styles.skinControlGrid} data-disabled={skinControlsDisabled}>
                <FieldRow label="背景适配">
                  <SelectField
                    value={current?.backgroundFit ?? "cover"}
                    disabled={skinControlsDisabled}
                    onChange={(event) => update({ backgroundFit: event.target.value as BackgroundFit })}
                  >
                    {BACKGROUND_FITS.map((fit) => (
                      <option value={fit} key={fit}>{fit === "cover" ? "填充窗口" : "完整适应"}</option>
                    ))}
                  </SelectField>
                </FieldRow>
                <FieldRow label="背景遮罩">
                  <RangeNumberField
                    label="背景遮罩"
                    value={current?.backgroundDim ?? 24}
                    min={0}
                    max={100}
                    unit="%"
                    disabled={skinControlsDisabled}
                    onChange={(backgroundDim) => update({ backgroundDim })}
                  />
                </FieldRow>
                <FieldRow label="背景模糊">
                  <RangeNumberField
                    label="背景模糊"
                    value={current?.backgroundBlur ?? 6}
                    min={0}
                    max={24}
                    unit="px"
                    disabled={skinControlsDisabled || Boolean(current?.reduceTransparency)}
                    onChange={(backgroundBlur) => update({ backgroundBlur })}
                  />
                </FieldRow>
                <FieldRow label="面板透明度">
                  <RangeNumberField
                    label="面板透明度"
                    value={current?.panelOpacity ?? 86}
                    min={0}
                    max={100}
                    unit="%"
                    disabled={skinControlsDisabled || Boolean(current?.reduceTransparency)}
                    onChange={(panelOpacity) => update({ panelOpacity })}
                  />
                </FieldRow>
              </div>
              {current?.background && current.reduceTransparency && (
                <p className={styles.skinAccessibilityNote}>
                  已启用“减少透明效果”，背景仍保留，但内容面板以不透明方式显示。
                </p>
              )}
            </div>
          </div>
        </SettingsCard>
      </div>
    </PageScaffold>
  );
}
