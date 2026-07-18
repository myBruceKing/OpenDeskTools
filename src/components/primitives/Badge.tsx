import {
  CheckmarkCircle20Regular,
  Info16Regular,
  SubtractCircle20Regular,
  Warning20Regular
} from "@fluentui/react-icons";
import type { HotkeyState } from "../../app/overviewModel";
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
    <span className={classes} title={detail ?? undefined}>
      <Icon aria-hidden="true" />
      <span className={styles.statusLabel}>{statusCopy[state]}</span>
      {state === "conflict" && <Info16Regular className={styles.statusInfo} aria-hidden="true" />}
      {detail && <small className={styles.statusDetail}>{detail}</small>}
    </span>
  );
}

type TagTone = "blue" | "green" | "warning";

export function TagBadge({ children, tone = "blue" }: { children: string; tone?: TagTone }) {
  return <span className={[styles.tagBadge, styles[`tag${tone}`]].join(" ")}>{children}</span>;
}
