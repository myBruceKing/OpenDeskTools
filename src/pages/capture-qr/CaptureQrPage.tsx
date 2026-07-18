import {
  Crop28Regular,
  ScanQrCode24Regular,
  Screenshot24Regular
} from "@fluentui/react-icons";
import { useState } from "react";
import { PageScaffold } from "../../components/layout/PageScaffold";
import { SettingsCard } from "../../components/layout/SettingsCard";
import { SectionTitle } from "../../components/patterns/Section";
import { SelectField, TextField } from "../../components/primitives/Field";
import { FieldRow, SwitchRow } from "../static/SettingsRows";
import styles from "../static/SettingsPages.module.css";

export function CaptureQrPage() {
  const [crosshairColor, setCrosshairColor] = useState("蓝色");
  const [captureDelay, setCaptureDelay] = useState("0");
  const [saveFormat, setSaveFormat] = useState("PNG");
  const [qrTolerance, setQrTolerance] = useState("中（推荐）");

  return (
    <PageScaffold title="截图与二维码" description="F1 区域截图，F3 屏幕贴图，F4 剪贴板二维码互转。">
      <div className={styles.captureGrid}>
        <SettingsCard fill>
          <div className={styles.featureTitle}>
            <Crop28Regular className={styles.featureTitleIcon} aria-hidden="true" />
            <SectionTitle>F1 区域截图</SectionTitle>
          </div>
          <div className={styles.screenshotPreview}>
            <span className={styles.cropBox} />
            <span className={styles.cropSize}>960 × 540</span>
          </div>
          <SwitchRow title="复制到剪贴板" description="截图完成后写入剪贴板" checked />
          <SwitchRow title="保存到文件" description="保存到默认截图目录" checked />
          <SwitchRow title="截图后识别二维码" description="截图完成后交给 F4 识别流程" checked />
        </SettingsCard>
        <SettingsCard fill>
          <div className={styles.featureTitle}>
            <ScanQrCode24Regular className={styles.featureTitleIcon} aria-hidden="true" />
            <SectionTitle>F4 剪贴板二维码</SectionTitle>
          </div>
          <div className={styles.qrFlow}>
            <span>剪贴板文本</span>
            <span className={styles.arrow}>→</span>
            <span className={styles.qrBox}>▦</span>
            <span>生成二维码图片</span>
          </div>
          <div className={styles.qrFlow}>
            <span>剪贴板图片</span>
            <span className={styles.arrow}>→</span>
            <span className={styles.qrBox}>⌗</span>
            <span>识别二维码文本</span>
          </div>
          <SwitchRow title="保留原始剪贴板内容" description="处理失败时不覆盖原内容" checked />
        </SettingsCard>
        <SettingsCard fill>
          <div className={styles.featureTitle}>
            <Screenshot24Regular className={styles.featureTitleIcon} aria-hidden="true" />
            <SectionTitle>F3 屏幕贴图</SectionTitle>
          </div>
          <div className={styles.pinPreview}>图片贴到屏幕上方，可拖动、缩放、关闭。</div>
          <SwitchRow title="多个贴图实例" description="允许同时保留多个贴图窗口" checked />
          <SwitchRow title="无图片时紧凑提示" description="不打开大面板，不打断工作流" checked />
        </SettingsCard>
      </div>
      <SettingsCard>
        <SectionTitle>截图设置</SectionTitle>
        <div className={styles.formGrid}>
          <FieldRow label="十字线颜色">
            <SelectField value={crosshairColor} onChange={(event) => setCrosshairColor(event.target.value)}>
              <option>蓝色</option>
              <option>红色</option>
              <option>绿色</option>
              <option>白色</option>
            </SelectField>
          </FieldRow>
          <FieldRow label="截图延迟">
            <TextField
              type="number"
              min="0"
              max="5000"
              value={captureDelay}
              unit="毫秒"
              onChange={(event) => setCaptureDelay(event.target.value)}
            />
          </FieldRow>
          <FieldRow label="保存格式">
            <SelectField value={saveFormat} onChange={(event) => setSaveFormat(event.target.value)}>
              <option>PNG</option>
              <option>JPG</option>
              <option>WebP</option>
            </SelectField>
          </FieldRow>
          <FieldRow label="识别容错度">
            <SelectField value={qrTolerance} onChange={(event) => setQrTolerance(event.target.value)}>
              <option>低</option>
              <option>中（推荐）</option>
              <option>高</option>
            </SelectField>
          </FieldRow>
        </div>
      </SettingsCard>
    </PageScaffold>
  );
}
