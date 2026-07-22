import {
  Crop28Regular,
  ScanQrCode24Regular,
  Screenshot24Regular
} from "@fluentui/react-icons";
import { useQrConversion } from "../../app/qrConversionModel";
import { PageScaffold } from "../../components/layout/PageScaffold";
import { SettingsCard } from "../../components/layout/SettingsCard";
import { SectionTitle } from "../../components/patterns/Section";
import { SelectField, TextField } from "../../components/primitives/Field";
import { Button } from "../../components/primitives/Button";
import { FieldRow, SwitchRow } from "../static/SettingsRows";
import styles from "../static/SettingsPages.module.css";

export function CaptureQrPage() {
  const unavailableValue = "—";
  const qr = useQrConversion();

  return (
    <PageScaffold title="截图与二维码" description="F4 使用内置剪贴板最新记录进行二维码互转；截图和贴图将在后续接入。">
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
          <SwitchRow title="复制到剪贴板" description="截图完成后写入剪贴板" checked={null} disabled />
          <SwitchRow title="保存到文件" description="保存到默认截图目录" checked={null} disabled />
          <SwitchRow title="截图后识别二维码" description="截图完成后交给 F4 识别流程" checked={null} disabled />
        </SettingsCard>
        <SettingsCard fill>
          <div className={styles.featureTitle}>
            <ScanQrCode24Regular className={styles.featureTitleIcon} aria-hidden="true" />
            <SectionTitle>F4 剪贴板二维码</SectionTitle>
          </div>
          <div className={styles.qrFlow}>
            <span>最新内部文本 / 链接</span>
            <span className={styles.arrow}>→</span>
            <span className={styles.qrBox}>▦</span>
            <span>生成二维码图片</span>
          </div>
          <div className={styles.qrFlow}>
            <span>最新内部图片</span>
            <span className={styles.arrow}>→</span>
            <span className={styles.qrBox}>⌗</span>
            <span>识别二维码文本</span>
          </div>
          <div className={styles.qrActionFooter}>
            <span>{qr.message ?? "结果会保存到内置历史，并尝试同步系统剪贴板。"}</span>
            <Button variant="primary" disabled={qr.pending} onClick={() => void qr.convertLatest()}>
              {qr.pending ? "正在转换" : "转换最新记录"}
            </Button>
          </div>
        </SettingsCard>
        <SettingsCard fill>
          <div className={styles.featureTitle}>
            <Screenshot24Regular className={styles.featureTitleIcon} aria-hidden="true" />
            <SectionTitle>F3 屏幕贴图</SectionTitle>
          </div>
          <div className={styles.pinPreview}>图片贴到屏幕上方，可拖动、缩放、关闭。</div>
          <SwitchRow title="多个贴图实例" description="允许同时保留多个贴图窗口" checked={null} disabled />
          <SwitchRow title="无图片时紧凑提示" description="不打开大面板，不打断工作流" checked={null} disabled />
        </SettingsCard>
      </div>
      <SettingsCard>
        <SectionTitle>截图设置</SectionTitle>
        <div className={styles.formGrid}>
          <FieldRow label="十字线颜色">
            <SelectField value="" disabled>
              <option value="">{unavailableValue}</option>
            </SelectField>
          </FieldRow>
          <FieldRow label="截图延迟">
            <TextField value={unavailableValue} disabled />
          </FieldRow>
          <FieldRow label="保存格式">
            <SelectField value="" disabled>
              <option value="">{unavailableValue}</option>
            </SelectField>
          </FieldRow>
          <FieldRow label="识别容错度">
            <SelectField value="" disabled>
              <option value="">{unavailableValue}</option>
            </SelectField>
          </FieldRow>
        </div>
      </SettingsCard>
    </PageScaffold>
  );
}
