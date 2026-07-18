import type { ReactNode } from "react";
import { ListRowDescription, ListRowTitle } from "../../components/patterns/ListRow";
import { Toggle } from "../../components/primitives/SelectionControl";
import styles from "./SettingsPages.module.css";

export function FieldRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className={styles.fieldRow}>
      <span>{label}</span>
      {children}
    </label>
  );
}

export function SwitchRow({
  title,
  description,
  checked = null,
  disabled = false,
  onChange
}: {
  title: string;
  description: string;
  checked?: boolean | null;
  disabled?: boolean;
  onChange?: (checked: boolean) => void;
}) {
  return (
    <div className={styles.switchRow}>
      <div>
        <ListRowTitle>{title}</ListRowTitle>
        <ListRowDescription>{description}</ListRowDescription>
      </div>
      <Toggle checked={checked} label={title} disabled={disabled || checked === null} onChange={onChange} />
    </div>
  );
}
