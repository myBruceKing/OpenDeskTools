import type { ReactNode } from "react";
import { useState } from "react";
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
  checked = true,
  disabled = false
}: {
  title: string;
  description: string;
  checked?: boolean | null;
  disabled?: boolean;
}) {
  const [value, setValue] = useState(checked === null ? false : checked);
  const toggleValue = checked === null ? null : value;

  return (
    <div className={styles.switchRow}>
      <div>
        <ListRowTitle>{title}</ListRowTitle>
        <ListRowDescription>{description}</ListRowDescription>
      </div>
      <Toggle checked={toggleValue} label={title} disabled={disabled || checked === null} onChange={setValue} />
    </div>
  );
}
