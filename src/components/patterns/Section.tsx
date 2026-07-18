import type { HTMLAttributes, ReactNode } from "react";
import styles from "./patterns.module.css";

type SectionProps = HTMLAttributes<HTMLElement> & {
  title?: string;
  subtitle?: string;
  action?: ReactNode;
};

export function Section({ title, subtitle, action, className = "", children, ...props }: SectionProps) {
  const classes = [styles.section, className].filter(Boolean).join(" ");

  return (
    <section className={classes} {...props}>
      {(title || subtitle || action) && (
        <header className={styles.sectionHeader}>
          <div>
            {title && <h2 className={styles.sectionTitle}>{title}</h2>}
            {subtitle && <p className={styles.sectionSubtitle}>{subtitle}</p>}
          </div>
          {action}
        </header>
      )}
      {children}
    </section>
  );
}

export function SectionTitle({ children, className = "" }: { children: ReactNode; className?: string }) {
  return <h2 className={[styles.sectionTitle, className].filter(Boolean).join(" ")}>{children}</h2>;
}
