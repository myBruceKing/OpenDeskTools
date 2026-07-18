import { forwardRef } from "react";
import type { HTMLAttributes } from "react";
import styles from "./patterns.module.css";

export const List = forwardRef<HTMLDivElement, HTMLAttributes<HTMLDivElement>>(function List(
  { className = "", ...props },
  ref
) {
  return <div ref={ref} className={[styles.list, className].filter(Boolean).join(" ")} {...props} />;
});

export function ListRow({ className = "", ...props }: HTMLAttributes<HTMLDivElement>) {
  return <div className={[styles.listRow, className].filter(Boolean).join(" ")} {...props} />;
}

export function ListRowTitle({ className = "", ...props }: HTMLAttributes<HTMLDivElement>) {
  return <div className={[styles.listRowTitle, className].filter(Boolean).join(" ")} {...props} />;
}

export function ListRowDescription({ className = "", ...props }: HTMLAttributes<HTMLDivElement>) {
  return <div className={[styles.listRowDescription, className].filter(Boolean).join(" ")} {...props} />;
}
