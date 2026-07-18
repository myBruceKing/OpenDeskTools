import styles from "./primitives.module.css";

type ToggleProps = {
  checked: boolean | null;
  label: string;
  disabled?: boolean;
  onChange?: (checked: boolean) => void;
};

export function Toggle({ checked, label, disabled = false, onChange }: ToggleProps) {
  if (checked === null) {
    return (
      <button
        className={[styles.toggle, styles.toggleUnavailable].join(" ")}
        type="button"
        aria-label={`${label}，状态不可用`}
        data-state="unavailable"
        disabled
      >
        <span className={styles.toggleThumb} />
      </button>
    );
  }

  const isOn = checked === true;
  const classes = [styles.toggle, isOn ? styles.toggleOn : ""].filter(Boolean).join(" ");
  const isDisabled = disabled || !onChange;

  return (
    <button
      className={classes}
      type="button"
      role="switch"
      aria-checked={isOn}
      aria-label={label}
      disabled={isDisabled}
      onClick={() => onChange?.(!isOn)}
    >
      <span className={styles.toggleThumb} />
    </button>
  );
}

type SegmentedOption<T extends string> = {
  value: T;
  label: string;
};

type SegmentedControlProps<T extends string> = {
  label: string;
  value: T;
  options: SegmentedOption<T>[];
  onChange: (value: T) => void;
  disabled?: boolean;
};

export function SegmentedControl<T extends string>({
  label,
  value,
  options,
  onChange,
  disabled = false
}: SegmentedControlProps<T>) {
  return (
    <div className={styles.segmented} role="radiogroup" aria-label={label}>
      {options.map((option) => (
        <button
          className={[styles.segmentedItem, option.value === value ? styles.segmentedItemActive : ""]
            .filter(Boolean)
            .join(" ")}
          type="button"
          role="radio"
          aria-checked={option.value === value}
          disabled={disabled}
          key={option.value}
          onClick={() => onChange(option.value)}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}
