import type { HTMLAttributes, ReactNode } from "react";
import styles from "./PageScaffold.module.css";

type PageScaffoldVariant = "standard" | "overview";

export type PageHeaderProps = {
  title: string;
  description: string;
  className?: string;
};

export function PageHeader({ title, description, className = "" }: PageHeaderProps) {
  return (
    <header className={[styles.pageHeader, className].filter(Boolean).join(" ")}>
      <h1 className={styles.title}>{title}</h1>
      <p className={styles.description}>{description}</p>
    </header>
  );
}

type PageScaffoldProps = Omit<HTMLAttributes<HTMLDivElement>, "title"> & {
  title: string;
  description: string;
  children: ReactNode;
  contentClassName?: string;
  variant?: PageScaffoldVariant;
};

export function PageScaffold({
  title,
  description,
  children,
  className = "",
  contentClassName = "",
  variant = "standard",
  ...props
}: PageScaffoldProps) {
  return (
    <div
      className={[
        styles.pageScaffold,
        variant === "overview" ? styles.pageScaffoldOverview : styles.pageScaffoldStandard,
        className
      ]
        .filter(Boolean)
        .join(" ")}
      {...props}
    >
      <PageHeader title={title} description={description} />
      <div
        className={[
          styles.pageBody,
          variant === "overview" ? styles.pageBodyOverview : styles.pageBodyStandard,
          contentClassName
        ]
          .filter(Boolean)
          .join(" ")}
      >
        {children}
      </div>
    </div>
  );
}
