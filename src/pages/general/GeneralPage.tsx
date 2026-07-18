import { Settings24Regular } from "@fluentui/react-icons";
import { PageScaffold } from "../../components/layout/PageScaffold";
import { SettingsCard } from "../../components/layout/SettingsCard";
import { SectionTitle } from "../../components/patterns/Section";
import { Button } from "../../components/primitives/Button";
import { TextField } from "../../components/primitives/Field";
import { FieldRow, SwitchRow } from "../static/SettingsRows";
import styles from "../static/SettingsPages.module.css";

type GeneralPageProps = {
  version: string | null;
  startupEnabled: boolean | null;
};

export function GeneralPage({ version, startupEnabled }: GeneralPageProps) {
  const unavailableValue = "—";

  return (
    <PageScaffold title="常规" description="应用启动、数据目录、更新和基础行为设置。">
      <div className={styles.generalGrid}>
        <SettingsCard fill>
          <div className={styles.featureTitle}>
            <Settings24Regular className={styles.featureTitleIcon} aria-hidden="true" />
            <SectionTitle>应用行为</SectionTitle>
          </div>
          <SwitchRow title="开机自启动" description="登录 Windows 后自动启动后台服务" checked={startupEnabled} disabled />
          <SwitchRow title="启动时最小化到托盘" description="不主动打断当前桌面" checked={null} disabled />
          <SwitchRow title="关闭按钮最小化到托盘" description="保持后台快捷键和剪贴板服务" checked={null} disabled />
        </SettingsCard>
        <SettingsCard fill>
          <SectionTitle>数据与隐私</SectionTitle>
          <FieldRow label="数据目录">
            <TextField value={unavailableValue} disabled />
          </FieldRow>
          <SwitchRow title="本地保存设置" description="设置和历史数据只保存在本机" checked={null} disabled />
          <SwitchRow title="崩溃日志" description="仅保存本地诊断日志" checked={null} disabled />
        </SettingsCard>
        <SettingsCard fill>
          <SectionTitle>更新</SectionTitle>
          <SwitchRow title="自动检查更新" description="启动后检查是否有新版本" checked={null} disabled />
          <FieldRow label="当前版本">
            <TextField value={version ?? unavailableValue} disabled />
          </FieldRow>
          <Button disabled>检查更新</Button>
        </SettingsCard>
      </div>
      <SettingsCard>
        <SectionTitle>配置导入导出</SectionTitle>
        <div className={styles.formGrid}>
          <Button disabled>导出设置</Button>
          <Button disabled>导入设置</Button>
          <Button disabled>打开数据目录</Button>
        </div>
      </SettingsCard>
    </PageScaffold>
  );
}
