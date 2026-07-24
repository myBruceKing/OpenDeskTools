import {
  Crop28Regular,
  Pin24Regular,
  ScanQrCode24Regular,
} from "@fluentui/react-icons";
import { useCaptureTools } from "../../app/captureToolsModel";
import { useQrConversion } from "../../app/qrConversionModel";
import { PageScaffold } from "../../components/layout/PageScaffold";
import { SettingsCard } from "../../components/layout/SettingsCard";
import { SectionTitle } from "../../components/patterns/Section";
import { Button } from "../../components/primitives/Button";
import styles from "../static/SettingsPages.module.css";

export function CaptureQrPage() {
  const tools = useCaptureTools();
  const qr = useQrConversion();

  return (
    <PageScaffold title="截图与二维码" description="F1 区域截图、F3 屏幕贴图和 F4 二维码转换均由本地能力完成。">
      <div className={styles.captureGrid}>
        <SettingsCard fill>
          <div className={styles.featureTitle}>
            <Crop28Regular className={styles.featureTitleIcon} aria-hidden="true" />
            <SectionTitle>F1 区域截图</SectionTitle>
          </div>
          <div className={styles.featureSummary}>
            <strong>冻结桌面后框选，不会截入选区层</strong>
            <span>支持多显示器、跨屏框选和物理像素尺寸提示。</span>
            <span>确认后保存到内置历史，并复制到系统剪贴板。</span>
          </div>
          <div className={styles.featureActionFooter}>
            <span aria-live="polite">
              {tools.message?.action === "screenshot"
                ? tools.message.text
                : "Esc 或右键取消，Enter 或双击确认。"}
            </span>
            <Button
              variant="primary"
              disabled={tools.pending !== null}
              onClick={() => void tools.startScreenshot()}
            >
              {tools.pending === "screenshot" ? "正在截图" : "开始截图"}
            </Button>
          </div>
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
          <div className={styles.featureActionFooter}>
            <span aria-live="polite">
              {qr.message ?? "结果会保存到内置历史，并尝试同步系统剪贴板。"}
            </span>
            <Button variant="primary" disabled={qr.pending} onClick={() => void qr.convertLatest()}>
              {qr.pending ? "正在转换" : "转换最新记录"}
            </Button>
          </div>
        </SettingsCard>
        <SettingsCard fill>
          <div className={styles.featureTitle}>
            <Pin24Regular className={styles.featureTitleIcon} aria-hidden="true" />
            <SectionTitle>F3 屏幕贴图</SectionTitle>
          </div>
          <div className={styles.featureSummary}>
            <strong>贴出历史中最近的一张图片</strong>
            <span>无边框置顶且不抢输入焦点，可同时保留多个实例。</span>
            <span>拖动位置，滚轮缩放，Ctrl + 滚轮调整透明度。</span>
          </div>
          <div className={styles.featureActionFooter}>
            <span aria-live="polite">
              {tools.message?.action === "pin"
                ? tools.message.text
                : "右键可恢复原始大小、调整透明度或关闭。"}
            </span>
            <Button
              variant="primary"
              disabled={tools.pending !== null}
              onClick={() => void tools.pinLatest()}
            >
              {tools.pending === "pin" ? "正在贴出" : "贴出最新图片"}
            </Button>
          </div>
        </SettingsCard>
      </div>
    </PageScaffold>
  );
}
