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
import styles from "../static/SettingsPages.module.css";

export function ThemePage() {
  const themeColors = ["#216bd9", "#7955c7", "#008b83", "#c7427a", "#e36a00", "#6d7782"];

  return (
    <PageScaffold title="主题" description="主题服务未接入；以下内容仅作视觉示意，暂不可修改。">
      <div className={styles.themeGrid}>
        <SettingsCard fill>
          <SectionTitle>主题设置</SectionTitle>
          <SegmentedControl
            label="主题模式"
            value="unavailable"
            options={[
              { value: "system", label: "跟随系统" },
              { value: "light", label: "浅色" },
              { value: "dark", label: "深色" }
            ]}
            onChange={() => undefined}
            disabled
          />
          <div className={styles.swatches}>
            {themeColors.map((color) => (
              <button
                className={styles.swatch}
                style={{ backgroundColor: color }}
                type="button"
                aria-label={`主题色视觉示意 ${color}`}
                key={color}
                disabled
              />
            ))}
          </div>
          <SwitchRow title="减少透明效果" description="降低半透明和模糊层级" checked={null} disabled />
          <FieldRow label="动画速度">
            <SelectField value="" disabled>
              <option value="">—</option>
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
