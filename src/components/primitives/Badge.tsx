import {
  CheckmarkCircle20Regular,
  SubtractCircle20Regular,
  Warning20Regular
} from "@fluentui/react-icons";
import type { HotkeyState } from "../../app/overviewModel";
import { HintTooltip } from "./HintTooltip";
import styles from "./primitives.module.css";

export function ShortcutBadge({ value }: { value: string | null }) {
  return <span className={styles.keycap}>{value ?? "—"}</span>;
}

const statusCopy: Record<HotkeyState, string> = {
  normal: "正常",
  conflict: "冲突",
  unavailable: "不可用",
  unknown: "未知"
};

export function StatusBadge({ state, detail }: { state: HotkeyState; detail: string | null }) {
  const Icon =
    state === "normal"
      ? CheckmarkCircle20Regular
      : state === "conflict"
        ? Warning20Regular
        : SubtractCircle20Regular;
  const classes = [styles.status, styles[state]].join(" ");

  return (
    <span className={classes}>
      <Icon aria-hidden="true" />
      <span className={styles.statusLabel}>{statusCopy[state]}</span>
      {detail && (
        <HintTooltip
          className={styles.statusInfo}
          symbol="i"
          label={`${statusCopy[state]}状态说明`}
          content={detail}
        />
      )}
    </span>
  );
}

type TagTone = "blue" | "green" | "warning";

export function TagBadge({ children, tone = "blue" }: { children: string; tone?: TagTone }) {
  return <span className={[styles.tagBadge, styles[`tag${tone}`]].join(" ")}>{children}</span>;
}
