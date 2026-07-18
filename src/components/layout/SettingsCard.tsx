import type { ComponentProps } from "react";
import { Section } from "../patterns/Section";
import styles from "./layout.module.css";

type SettingsCardProps = ComponentProps<typeof Section> & {
  fill?: boolean;
};

export function SettingsCard({ fill = false, className = "", children, ...props }: SettingsCardProps) {
  return (
    <Section
      className={[
        styles.settingsCard,
        fill ? styles.settingsCardFill : styles.settingsCardFixed,
        className
      ]
        .filter(Boolean)
        .join(" ")}
      {...props}
    >
      {children}
    </Section>
  );
}
