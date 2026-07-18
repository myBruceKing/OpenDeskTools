import type { FluentIcon } from "@fluentui/react-icons";
import type { HTMLAttributes } from "react";
import styles from "./patterns.module.css";

type MetricTone = "blue" | "green" | "orange" | "purple";

type MetricCardProps = HTMLAttributes<HTMLDivElement> & {
  icon: FluentIcon;
  label: string;
  value: string;
  tone: MetricTone;
};

export function MetricCard({ icon: Icon, label, value, tone, className = "", ...props }: MetricCardProps) {
  return (
    <div className={[styles.metricCard, className].filter(Boolean).join(" ")} {...props}>
      <span className={[styles.metricIcon, styles[`metricIcon${tone}`]].join(" ")} aria-hidden="true">
        <Icon aria-hidden={true} />
      </span>
      <span className={styles.metricCopy}>
        <strong className={styles.metricValue}>{value}</strong>
        <span className={styles.metricLabel}>{label}</span>
      </span>
    </div>
  );
}
