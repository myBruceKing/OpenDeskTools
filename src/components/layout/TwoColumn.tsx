import type { CSSProperties, HTMLAttributes } from "react";
import styles from "./layout.module.css";

type TwoColumnProps = HTMLAttributes<HTMLDivElement> & {
  sideWidth?: string;
  gap?: string;
};

type LayoutVariables = CSSProperties & {
  "--two-column-side-override"?: string;
  "--two-column-gap-override"?: string;
  "--split-pane-left"?: string;
  "--split-pane-gap"?: string;
  "--three-column-template"?: string;
};

export function TwoColumn({
  sideWidth,
  gap,
  style,
  className = "",
  ...props
}: TwoColumnProps) {
  const variables: LayoutVariables = {
    ...(sideWidth ? { "--two-column-side-override": sideWidth } : {}),
    ...(gap ? { "--two-column-gap-override": gap } : {}),
    ...style
  };

  return (
    <div
      className={[styles.twoColumn, className].filter(Boolean).join(" ")}
      style={variables}
      {...props}
    />
  );
}

type SplitPaneProps = HTMLAttributes<HTMLDivElement> & {
  left?: string;
  gap?: string;
};

export function SplitPane({ left, gap, style, className = "", ...props }: SplitPaneProps) {
  const variables: LayoutVariables = {
    ...(left ? { "--split-pane-left": left } : {}),
    ...(gap ? { "--split-pane-gap": gap } : {}),
    ...style
  };

  return <div className={[styles.splitPane, className].filter(Boolean).join(" ")} style={variables} {...props} />;
}

type ThreeColumnProps = HTMLAttributes<HTMLDivElement> & {
  columns?: string;
};

export function ThreeColumn({ columns, style, className = "", ...props }: ThreeColumnProps) {
  const variables: LayoutVariables = {
    ...(columns ? { "--three-column-template": columns } : {}),
    ...style
  };

  return <div className={[styles.threeColumn, className].filter(Boolean).join(" ")} style={variables} {...props} />;
}
