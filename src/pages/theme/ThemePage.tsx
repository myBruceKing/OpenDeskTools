import {
  Color24Regular,
  Info20Regular,
  Rocket24Regular,
  Screenshot24Regular
} from "@fluentui/react-icons";
import type { CSSProperties } from "react";
import { useState } from "react";
import { PageScaffold } from "../../components/layout/PageScaffold";
import { SettingsCard } from "../../components/layout/SettingsCard";
import { ListRowDescription, ListRowTitle } from "../../components/patterns/ListRow";
import { SectionTitle } from "../../components/patterns/Section";
import { SelectField } from "../../components/primitives/Field";
import { SegmentedControl } from "../../components/primitives/SelectionControl";
import { FieldRow, SwitchRow } from "../static/SettingsRows";
import styles from "../static/SettingsPages.module.css";

export function ThemePage() {
  const [themeMode, setThemeMode] = useState<"system" | "light" | "dark">("system");
  const [themeColor, setThemeColor] = useState("#216bd9");
  const [animationSpeed, setAnimationSpeed] = useState("中（默认）");
  const themeColors = ["#216bd9", "#7955c7", "#008b83", "#c7427a", "#e36a00", "#6d7782"];

  return (
    <PageScaffold title="主题" description="设置主窗口、简洁弹窗、截图遮罩和快速弹窗的主题表现。">
      <div className={styles.themeGrid}>
        <SettingsCard fill>
          <SectionTitle>主题设置</SectionTitle>
          <SegmentedControl
            label="主题模式"
            value={themeMode}
            options={[
              { value: "system", label: "跟随系统" },
              { value: "light", label: "浅色" },
              { value: "dark", label: "深色" }
            ]}
            onChange={setThemeMode}
          />
          <div className={styles.swatches}>
            {themeColors.map((color) => (
              <button
                className={[styles.swatch, themeColor === color ? styles.swatchActive : ""].filter(Boolean).join(" ")}
                style={{ backgroundColor: color }}
                type="button"
                aria-label={`主题色 ${color}`}
                key={color}
                onClick={() => setThemeColor(color)}
              />
            ))}
          </div>
          <SwitchRow title="减少透明效果" description="降低半透明和模糊层级" checked={false} />
          <FieldRow label="动画速度">
            <SelectField value={animationSpeed} onChange={(event) => setAnimationSpeed(event.target.value)}>
              <option>慢</option>
              <option>中（默认）</option>
              <option>快</option>
            </SelectField>
          </FieldRow>
        </SettingsCard>
        <SettingsCard fill>
          <div className={styles.featureTitle}>
            <Color24Regular className={styles.featureTitleIcon} aria-hidden="true" />
            <SectionTitle>主页面主题</SectionTitle>
          </div>
          <div className={styles.themePreviewWindow} style={{ "--theme-preview-color": themeColor } as CSSProperties}>
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
          <div className={styles.themePreviewToast} style={{ "--theme-preview-color": themeColor } as CSSProperties}>
            <span className={styles.themePreviewDot} />
            <div>
              <strong>操作成功</strong>
              <span>已复制到剪贴板</span>
            </div>
          </div>
        </SettingsCard>
      </div>
      <SettingsCard>
        <SectionTitle>临时界面主题预览</SectionTitle>
        <div className={styles.themeSurfaceGrid}>
          <div className={styles.themeSurfaceCard} style={{ "--theme-preview-color": themeColor } as CSSProperties}>
            <Screenshot24Regular className={styles.featureTitleIcon} aria-hidden="true" />
            <div>
              <ListRowTitle>截图遮罩</ListRowTitle>
              <ListRowDescription>选区边框、尺寸标签和遮罩色跟随主题。</ListRowDescription>
            </div>
          </div>
          <div className={styles.themeSurfaceCard} style={{ "--theme-preview-color": themeColor } as CSSProperties}>
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
