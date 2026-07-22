import { Settings24Regular } from "@fluentui/react-icons";
import { PageScaffold } from "../../components/layout/PageScaffold";
import { SettingsCard } from "../../components/layout/SettingsCard";
import { SectionTitle } from "../../components/patterns/Section";
import { Button } from "../../components/primitives/Button";
import { TextField } from "../../components/primitives/Field";
import { InlineNotice } from "../../components/primitives/InlineNotice";
import { useGeneralSettings } from "../../app/useGeneralSettings";
import { FieldRow, SwitchRow } from "../static/SettingsRows";
import styles from "../static/SettingsPages.module.css";

export function GeneralPage() {
  const { state, setToggle, selectAndMigrateDataDirectory } = useGeneralSettings();
  const { viewModel, pending, error, dataDirectoryMigration } = state;
  const unavailableValue = "—";
  const busy = pending !== null;

  return (
    <PageScaffold title="常规" description="应用启动、数据目录、更新和基础行为设置。">
      <div className={styles.generalGrid}>
        <SettingsCard fill>
          <div className={styles.featureTitle}>
            <Settings24Regular className={styles.featureTitleIcon} aria-hidden="true" />
            <SectionTitle>应用行为</SectionTitle>
          </div>
          {error ? <InlineNotice variant="error">{error}</InlineNotice> : null}
          <SwitchRow
            title="开机自启动"
            description="登录 Windows 后自动在后台启动（无感知，仅驻留托盘）"
            checked={viewModel.autostartEnabled}
            disabled={busy || viewModel.autostartEnabled === null}
            onChange={(checked) => void setToggle("autostart", checked)}
          />
          <SwitchRow
            title="启动时最小化到托盘"
            description="正常启动时不弹出主窗口，只驻留托盘"
            checked={viewModel.startMinimized}
            disabled={busy || viewModel.startMinimized === null}
            onChange={(checked) => void setToggle("startMinimized", checked)}
          />
          <SwitchRow
            title="关闭按钮最小化到托盘"
            description="关闭主窗口时保留后台快捷键和剪贴板服务；关闭后从托盘重新打开"
            checked={viewModel.closeToTray}
            disabled={busy || viewModel.closeToTray === null}
            onChange={(checked) => void setToggle("closeToTray", checked)}
          />
        </SettingsCard>
        <SettingsCard fill>
          <SectionTitle>数据与隐私</SectionTitle>
          <FieldRow label="数据目录">
            <TextField
              aria-label="数据目录"
              value={viewModel.dataDirectory ?? unavailableValue}
              disabled
            />
          </FieldRow>
          <Button onClick={() => void selectAndMigrateDataDirectory()} disabled={viewModel.dataDirectory === null || busy}>
            选择路径
          </Button>
          {dataDirectoryMigration ? (
            <InlineNotice variant="success">
              数据已复制到 {dataDirectoryMigration.dataDirectory}。请退出并重新启动后生效；原目录保留作恢复备份。
            </InlineNotice>
          ) : null}
          <InlineNotice variant="info">设置、剪贴板历史和图片仅保存在本机，不会上传云端。</InlineNotice>
          <SwitchRow
            title="本地崩溃诊断日志"
            description="发生 Rust 崩溃时仅在数据目录的 diagnostics 文件夹写入诊断报告"
            checked={viewModel.crashDiagnosticsEnabled}
            disabled={busy || viewModel.crashDiagnosticsEnabled === null}
            onChange={(checked) => void setToggle("crashDiagnostics", checked)}
          />
        </SettingsCard>
        <SettingsCard fill>
          <SectionTitle>版本</SectionTitle>
          <FieldRow label="当前版本">
            <TextField value={viewModel.version ?? unavailableValue} disabled />
          </FieldRow>
          <InlineNotice variant="info">当前版本不包含在线更新机制；请使用新安装包完成更新。</InlineNotice>
        </SettingsCard>
      </div>
    </PageScaffold>
  );
}
