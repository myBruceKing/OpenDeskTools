import type { HTMLAttributes } from "react";
import styles from "./layout.module.css";

type ToolbarRowProps = HTMLAttributes<HTMLDivElement> & {
  layout?: "row" | "grid";
  wrap?: boolean;
};

export function ToolbarRow({ layout = "row", wrap = false, className = "", children, ...props }: ToolbarRowProps) {
  return (
    <div
      className={[
        styles.toolbarRow,
        layout === "grid" ? styles.toolbarRowGrid : "",
        wrap ? styles.toolbarRowWrap : "",
        className
      ]
        .filter(Boolean)
        .join(" ")}
      {...props}
    >
      {children}
    </div>
  );
}
