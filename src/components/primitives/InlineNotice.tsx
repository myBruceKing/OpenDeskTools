import type { ReactNode } from "react";
import styles from "./primitives.module.css";

export type InlineNoticeVariant = "info" | "warning" | "error" | "pending" | "success";

export type InlineNoticeRole = "status" | "alert" | "note";

const VARIANT_CLASS: Record<InlineNoticeVariant, string> = {
  info: styles.inlineNoticeInfo,
  warning: styles.inlineNoticeWarning,
  error: styles.inlineNoticeError,
  pending: styles.inlineNoticePending,
  success: styles.inlineNoticeSuccess
};

const VARIANT_ROLE: Record<InlineNoticeVariant, InlineNoticeRole> = {
  info: "note",
  warning: "status",
  error: "alert",
  pending: "status",
  success: "status"
};

type InlineNoticeProps = {
  variant: InlineNoticeVariant;
  children: ReactNode;
  /** Overrides the default ARIA role derived from the variant. */
  role?: InlineNoticeRole;
  className?: string;
};

/**
 * A single-line, bordered status banner shared across settings surfaces.
 * Encapsulates the previously duplicated inline status/notice styling so
 * info / warning / error / pending / success messages stay visually consistent.
 */
export function InlineNotice({ variant, children, role, className }: InlineNoticeProps) {
  const classNames = [styles.inlineNotice, VARIANT_CLASS[variant], className]
    .filter(Boolean)
    .join(" ");
  return (
    <div className={classNames} role={role ?? VARIANT_ROLE[variant]}>
      {children}
    </div>
  );
}
