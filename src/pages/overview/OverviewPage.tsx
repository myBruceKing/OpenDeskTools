import {
  ArrowTrending24Regular,
  Calendar32Regular,
  Clock32Regular,
  Desktop32Regular
} from "@fluentui/react-icons";
import {
  getToolWheelShortcutLabel,
  type OverviewLoadState,
  type OverviewHotkeyViewModel,
  type OverviewViewModel,
  type ServiceState
} from "../../app/overviewModel";
import { useQuickLaunchViewModel } from "../../app/quickLaunchModel";
import { PageScaffold } from "../../components/layout/PageScaffold";
import { TwoColumn } from "../../components/layout/TwoColumn";
import { Button } from "../../components/primitives/Button";
import { HintTooltip } from "../../components/primitives/HintTooltip";
import { HotkeyList } from "../../components/patterns/HotkeyList";
import { MetricCard } from "../../components/patterns/MetricCard";
import { Section, SectionTitle } from "../../components/patterns/Section";
import { ToolMenuPreview } from "../../components/patterns/ToolMenuPreview";
import styles from "./OverviewPage.module.css";

type OverviewPageProps = {
  viewModel: OverviewViewModel;
  loadState: OverviewLoadState;
  onRetry: () => Promise<void>;
};

const serviceLabels: Record<ServiceState, string> = {
  running: "运行中",
  starting: "启动中",
  stopped: "已停止",
  error: "异常",
  unknown: "未知"
};

function Summary({ viewModel }: Pick<OverviewPageProps, "viewModel">) {
  const startupLabel =
    viewModel.startupEnabled === null
      ? "—"
      : viewModel.startupEnabled
        ? "开机自启动"
        : "手动启动";
  const serviceDotClasses = [
    styles.summaryDot,
    viewModel.serviceState === "running" ? styles.summaryDotRunning : "",
    viewModel.serviceState === "error" ? styles.summaryDotError : ""
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <Section className={styles.summary} aria-label="服务摘要">
      <div className={styles.summaryCell}>
        <div>
          <div className={styles.summaryLabel}>服务状态</div>
          <div className={styles.summaryValue}>
            <span className={serviceDotClasses} aria-hidden="true" />
            {serviceLabels[viewModel.serviceState]}
          </div>
        </div>
      </div>
      <div className={styles.summaryCell}>
        <div>
          <div className={styles.summaryLabel}>启动方式</div>
          <div className={styles.summaryValue}>{startupLabel}</div>
        </div>
        <Button variant="text" disabled>
          管理
        </Button>
      </div>
      <div className={styles.summaryCell}>
        <div>
          <div className={styles.summaryLabel}>版本</div>
          <div className={styles.summaryValue}>{viewModel.version ?? "—"}</div>
        </div>
        <Button variant="text" disabled>
          检查更新
        </Button>
      </div>
    </Section>
  );
}

function HotkeyPanel({
  hotkeys,
  loadState,
  onRetry
}: {
  hotkeys: OverviewHotkeyViewModel[];
  loadState: OverviewLoadState;
  onRetry: () => Promise<void>;
}) {
  return (
    <section className={styles.hotkeyPanel} aria-label="全局快捷键">
      <SectionTitle className={styles.hotkeyTitle}>全局快捷键</SectionTitle>
      {loadState === "ready" ? (
        <HotkeyList hotkeys={hotkeys} density="full" toggleDisabled editDisabled />
      ) : (
        <div className={styles.overviewDataState} role={loadState === "error" ? "alert" : "status"}>
          <span>
            {loadState === "loading"
              ? "正在读取快捷键运行状态…"
              : "快捷键概览读取失败，未使用“未绑定”代替真实状态。"}
          </span>
          {loadState === "error" && (
            <Button size="inline" onClick={() => void onRetry()}>
              重试
            </Button>
          )}
        </div>
      )}
    </section>
  );
}

function ToolWheelPanel({
  hotkeys,
  loadState
}: {
  hotkeys: OverviewHotkeyViewModel[];
  loadState: OverviewLoadState;
}) {
  const { previewItems } = useQuickLaunchViewModel();
  const subtitle =
    loadState === "ready"
      ? getToolWheelShortcutLabel(hotkeys)
      : loadState === "loading"
        ? "正在读取快捷键状态…"
        : "快捷键状态不可用";

  return (
    <Section
      className={styles.toolPanel}
      title="工具盘预览"
      subtitle={subtitle}
      action={<HintTooltip content="这是工具盘预览。应用服务接入后，顺序会跟随快速启动页面里的固定项变化。" />}
      aria-label="工具盘预览"
    >
      <div className={styles.toolPreviewBody}>
        <ToolMenuPreview variant="wheel" size="overview" fit="container" className={styles.wheelFrame} items={previewItems} />
      </div>
    </Section>
  );
}

function formatCount(value: number | null) {
  return value === null ? "—" : new Intl.NumberFormat("zh-CN").format(value);
}

function formatSavedTime(seconds: number | null) {
  if (seconds === null) {
    return "—";
  }

  if (seconds === 0) {
    return "0 分钟";
  }

  if (seconds < 60) {
    return "< 1 分钟";
  }

  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) {
    return `${minutes} 分钟`;
  }

  const hours = minutes / 60;
  return `${Number.isInteger(hours) ? hours : hours.toFixed(1)} 小时`;
}

function StatsPanel({ statistics }: Pick<OverviewViewModel, "statistics">) {
  const statItems = [
    {
      label: "今日触发次数",
      value: formatCount(statistics.todayTriggers),
      icon: Desktop32Regular,
      tone: "blue" as const
    },
    {
      label: "本周触发次数",
      value: formatCount(statistics.weekTriggers),
      icon: Calendar32Regular,
      tone: "green" as const
    },
    {
      label: "本月触发次数",
      value: formatCount(statistics.monthTriggers),
      icon: ArrowTrending24Regular,
      tone: "orange" as const
    },
    {
      label: "节省时间（本月）",
      value: formatSavedTime(statistics.savedSecondsThisMonth),
      icon: Clock32Regular,
      tone: "purple" as const
    }
  ];

  return (
    <Section className={styles.statsPanel} aria-label="使用统计">
      <div className={styles.statsHeader}>
        <div>
          <SectionTitle>使用统计</SectionTitle>
        </div>
        <HintTooltip content="只统计实际执行成功的工具操作，数据保存在本机。节省时间按不同工具的保守估算累计。" />
      </div>
      <div className={styles.statsGrid}>
        {statItems.map((item) => (
          <MetricCard className={styles.statCard} key={item.label} {...item} />
        ))}
      </div>
    </Section>
  );
}

export function OverviewPage({ viewModel, loadState, onRetry }: OverviewPageProps) {
  return (
    <PageScaffold
      title="概览"
      description="查看后台服务、全局快捷键、工具盘和使用统计。"
      variant="overview"
    >
      <Summary viewModel={viewModel} />
      <TwoColumn className={styles.middle} sideWidth="minmax(270px, 34%)">
        <HotkeyPanel
          hotkeys={viewModel.hotkeys}
          loadState={loadState}
          onRetry={onRetry}
        />
        <ToolWheelPanel hotkeys={viewModel.hotkeys} loadState={loadState} />
      </TwoColumn>
      <StatsPanel statistics={viewModel.statistics} />
    </PageScaffold>
  );
}
