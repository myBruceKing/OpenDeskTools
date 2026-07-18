import { useEffect, useRef, useState, type KeyboardEvent, type InputHTMLAttributes, type ReactNode, type SelectHTMLAttributes, type TextareaHTMLAttributes } from "react";
import styles from "./primitives.module.css";

type SearchFieldProps = Omit<InputHTMLAttributes<HTMLInputElement>, "type"> & {
  icon?: ReactNode;
  shortcut?: string;
};

export function SearchField({ icon, shortcut, className = "", ...props }: SearchFieldProps) {
  return (
    <label className={[styles.searchField, className].filter(Boolean).join(" ")}>
      {icon && <span className={styles.fieldIcon}>{icon}</span>}
      <input type="search" {...props} />
      {shortcut && <kbd>{shortcut}</kbd>}
    </label>
  );
}

type TextFieldProps = InputHTMLAttributes<HTMLInputElement> & {
  unit?: string;
};

export function TextField({ unit, className = "", ...props }: TextFieldProps) {
  return (
    <span className={[styles.fieldWrap, className].filter(Boolean).join(" ")}>
      <input {...props} />
      {unit && <span className={styles.fieldUnit}>{unit}</span>}
    </span>
  );
}

export function SelectField({ className = "", children, ...props }: SelectHTMLAttributes<HTMLSelectElement>) {
  return (
    <span className={[styles.fieldWrap, className].filter(Boolean).join(" ")}>
      <select {...props}>{children}</select>
    </span>
  );
}

export function TextAreaField({ className = "", ...props }: TextareaHTMLAttributes<HTMLTextAreaElement>) {
  return <textarea className={[styles.textArea, className].filter(Boolean).join(" ")} {...props} />;
}

const modifierLabels = {
  ctrlKey: "Ctrl",
  altKey: "Alt",
  shiftKey: "Shift",
  metaKey: "Win"
} as const;

const keyLabels: Record<string, string> = {
  " ": "Space",
  ArrowUp: "↑",
  ArrowDown: "↓",
  ArrowLeft: "←",
  ArrowRight: "→",
  Escape: "Esc"
};

const modifierKeyLabels: Record<string, string> = {
  Control: "Ctrl",
  Alt: "Alt",
  Shift: "Shift",
  Meta: "Win"
};

function normalizeShortcutKey(event: KeyboardEvent<HTMLElement>) {
  const key = keyLabels[event.key] ?? event.key;

  if (modifierKeyLabels[event.key]) {
    return null;
  }

  const parts: string[] = Object.entries(modifierLabels)
    .filter(([flag]) => event[flag as keyof typeof modifierLabels])
    .map(([, label]) => label);

  parts.push(key.length === 1 ? key.toUpperCase() : key);
  return [...new Set(parts)].join("+");
}

type ShortcutCaptureFieldProps = {
  value: string;
  label: string;
  placeholder?: string;
  onChange: (value: string) => void;
  autoFocus?: boolean;
};

export function ShortcutCaptureField({
  value,
  label,
  placeholder = "按下快捷键",
  onChange,
  autoFocus = false
}: ShortcutCaptureFieldProps) {
  const tokens = value.trim().split(/\s+/).filter(Boolean);
  const [pendingModifiers, setPendingModifiers] = useState<string[]>([]);
  const modifierChordConsumedRef = useRef(false);
  const captureRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!autoFocus) {
      return undefined;
    }
    const frame = window.requestAnimationFrame(() => captureRef.current?.focus());
    return () => window.cancelAnimationFrame(frame);
  }, [autoFocus]);

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    event.preventDefault();
    event.stopPropagation();

    if (event.key === "Backspace") {
      onChange(tokens.slice(0, -1).join(" "));
      return;
    }

    if (event.key === "Delete") {
      onChange("");
      return;
    }

    if (event.key === "Tab") {
      return;
    }

    if (event.repeat) {
      return;
    }

    const modifierLabel = modifierKeyLabels[event.key];
    if (modifierLabel) {
      setPendingModifiers((current) => (current.includes(modifierLabel) ? current : [...current, modifierLabel]));
      return;
    }

    const nextToken = normalizeShortcutKey(event);
    if (!nextToken) {
      return;
    }

    if (pendingModifiers.length > 0 || event.ctrlKey || event.altKey || event.shiftKey || event.metaKey) {
      modifierChordConsumedRef.current = true;
      setPendingModifiers([]);
    }

    onChange([...tokens, nextToken].join(" "));
  };

  const handleKeyUp = (event: KeyboardEvent<HTMLDivElement>) => {
    event.preventDefault();
    event.stopPropagation();

    const modifierLabel = modifierKeyLabels[event.key];
    if (!modifierLabel) {
      return;
    }

    setPendingModifiers((current) => current.filter((label) => label !== modifierLabel));

    if (!modifierChordConsumedRef.current) {
      onChange([...tokens, modifierLabel].join(" "));
    }

    if (!event.ctrlKey && !event.altKey && !event.shiftKey && !event.metaKey) {
      modifierChordConsumedRef.current = false;
    }
  };

  const removeAt = (index: number) => {
    onChange(tokens.filter((_, tokenIndex) => tokenIndex !== index).join(" "));
    window.requestAnimationFrame(() => captureRef.current?.focus());
  };
  const visiblePendingModifiers = modifierChordConsumedRef.current ? [] : pendingModifiers;

  return (
    <div
      ref={captureRef}
      className={styles.shortcutCapture}
      role="group"
      tabIndex={0}
      aria-label={label}
      onKeyDown={handleKeyDown}
      onKeyUp={handleKeyUp}
      onMouseDown={(event) => {
        event.preventDefault();
        event.currentTarget.focus();
      }}
    >
      {tokens.length > 0 || visiblePendingModifiers.length > 0 ? (
        <div className={styles.shortcutCaptureTokens}>
          {tokens.map((token, index) => (
            <span className={styles.shortcutCaptureToken} key={`${token}-${index}`}>
              {token}
              <button
                className={styles.shortcutCaptureRemove}
                type="button"
                aria-label={`删除 ${token}`}
                onMouseDown={(event) => {
                  event.preventDefault();
                  event.stopPropagation();
                }}
                onClick={(event) => {
                  event.stopPropagation();
                  removeAt(index);
                }}
              >
                ×
              </button>
            </span>
          ))}
          {visiblePendingModifiers.map((token) => (
            <span className={[styles.shortcutCaptureToken, styles.shortcutCapturePendingToken].join(" ")} key={`pending-${token}`}>
              {token}
            </span>
          ))}
        </div>
      ) : (
        <span className={styles.shortcutCapturePlaceholder}>{placeholder}</span>
      )}
    </div>
  );
}
