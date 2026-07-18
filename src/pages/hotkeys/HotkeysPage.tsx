import type { OverviewHotkeyViewModel } from "../../app/overviewModel";
import { PageScaffold } from "../../components/layout/PageScaffold";
import { SettingsCard } from "../../components/layout/SettingsCard";
import { HotkeyList } from "../../components/patterns/HotkeyList";
import { SectionTitle } from "../../components/patterns/Section";
import { HintTooltip } from "../../components/primitives/HintTooltip";
import { SwitchRow } from "../static/SettingsRows";
import styles from "../static/SettingsPages.module.css";

export function HotkeysPage({ hotkeys }: { hotkeys: OverviewHotkeyViewModel[] }) {
  return (
    <PageScaffold title="快捷键" description="集中查看快捷键状态。系统注册服务未接入时，运行状态显示为不可用。">
      <div className={styles.hotkeyLayout}>
        <SettingsCard fill>
          <div className={styles.panelHeader}>
            <SectionTitle>全局快捷键</SectionTitle>
            <HintTooltip symbol="i" content="快捷键绑定、启用状态和冲突结果只展示后端实际返回的数据。" />
          </div>
          <HotkeyList hotkeys={hotkeys} density="full" toggleDisabled editDisabled />
        </SettingsCard>
        <SettingsCard fill>
          <SectionTitle>冲突处理</SectionTitle>
          <div className={styles.optionStack}>
            <SwitchRow title="自动避让" description="检测冲突时切换到备用快捷键" checked={null} disabled />
            <SwitchRow title="提示我解决" description="冲突发生时在设置页展示提示" checked={null} disabled />
            <SwitchRow title="保持当前设置" description="不自动修改用户已设快捷键" checked={null} disabled />
          </div>
        </SettingsCard>
      </div>
    </PageScaffold>
  );
}
