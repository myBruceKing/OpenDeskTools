import type { HTMLAttributes } from "react";
import styles from "./layout.module.css";

type PreviewFrameProps = HTMLAttributes<HTMLDivElement> & {
  inset?: boolean;
};

export function PreviewFrame({ inset = false, className = "", children, ...props }: PreviewFrameProps) {
  return (
    <div
      className={[styles.previewFrame, inset ? styles.previewFrameInset : "", className].filter(Boolean).join(" ")}
      {...props}
    >
      {children}
    </div>
  );
}
