import { forwardRef, useEffect, useImperativeHandle, useRef, useState, type FocusEvent, type KeyboardEvent, type InputHTMLAttributes, type ReactNode, type SelectHTMLAttributes, type TextareaHTMLAttributes } from "react";
import { canonicalHotkeyKeyFromInput } from "../../app/hotkeyKeyContract";
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

type RangeNumberFieldProps = {
  label: string;
  value: number;
  min: number;
  max: number;
  unit: string;
  disabled?: boolean;
  onChange: (value: number) => void;
};

export function RangeNumberField({
  label,
  value,
  min,
  max,
  unit,
  disabled = false,
  onChange
}: RangeNumberFieldProps) {
  const [draft, setDraft] = useState(String(value));
  const [rangeValue, setRangeValue] = useState(value);
  const pendingValueRef = useRef(value);
  const lastCommittedValueRef = useRef(value);

  useEffect(() => {
    setDraft(String(value));
    setRangeValue(value);
    pendingValueRef.current = value;
    lastCommittedValueRef.current = value;
  }, [value]);

  const normalizedValue = (candidate: number) =>
    Math.min(max, Math.max(min, Math.round(candidate)));

  const setPendingValue = (candidate: number) => {
    const next = normalizedValue(candidate);
    pendingValueRef.current = next;
    setRangeValue(next);
    setDraft(String(next));
  };

  const commitValue = (candidate: number) => {
    const next = normalizedValue(candidate);
    pendingValueRef.current = next;
    setRangeValue(next);
    setDraft(String(next));
    if (next !== lastCommittedValueRef.current) {
      lastCommittedValueRef.current = next;
      onChange(next);
    }
  };

  const commitDraft = () => {
    const parsed = Number(draft);
    if (!Number.isFinite(parsed)) {
      setDraft(String(value));
      return;
    }

    commitValue(parsed);
  };

  return (
    <div className={styles.rangeNumberField}>
      <input
        className={styles.rangeInput}
        type="range"
        min={min}
        max={max}
        step={1}
        value={rangeValue}
        disabled={disabled}
        aria-label={`${label}滑杆`}
        onChange={(event) => setPendingValue(Number(event.target.value))}
        onPointerUp={() => commitValue(pendingValueRef.current)}
        onPointerCancel={() => {
          pendingValueRef.current = value;
          setRangeValue(value);
          setDraft(String(value));
        }}
        onBlur={() => commitValue(pendingValueRef.current)}
        onKeyUp={(event) => {
          if (["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End", "PageUp", "PageDown"].includes(event.key)) {
            commitValue(pendingValueRef.current);
          }
        }}
      />
      <TextField
        className={styles.rangeValueField}
        type="number"
        min={min}
        max={max}
        step={1}
        value={draft}
        unit={unit}
        disabled={disabled}
        aria-label={`${label}数值`}
        onChange={(event) => setDraft(event.target.value)}
        onBlur={commitDraft}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            event.currentTarget.blur();
          } else if (event.key === "Escape") {
            event.preventDefault();
            setDraft(String(value));
            event.currentTarget.blur();
          }
        }}
      />
    </div>
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

const modifierKeyLabels: Record<string, string> = {
  Control: "Ctrl",
  Alt: "Alt",
  Shift: "Shift",
  Meta: "Win"
};

function normalizeShortcutKey(event: KeyboardEvent<HTMLElement>) {
  if (modifierKeyLabels[event.key]) {
    return null;
  }
  const key = canonicalHotkeyKeyFromInput(event.code, event.key);
  if (!key) {
    return null;
  }

  const parts: string[] = Object.entries(modifierLabels)
    .filter(([flag]) => event[flag as keyof typeof modifierLabels])
    .map(([, label]) => label);

  parts.push(key);
  return [...new Set(parts)].join("+");
}

type ShortcutCaptureFieldProps = {
  value: string;
  label: string;
  placeholder?: string;
  onChange: (value: string) => void;
  onAppendToken?: (token: string) => void;
  onCaptureStart?: () => void;
  onCaptureStop?: () => void;
  autoFocus?: boolean;
};

export type ShortcutCaptureFieldHandle = {
  acceptNativeToken: (token: string) => void;
};

export function shouldBypassShortcutCapture(key: string) {
  return key === "Tab" || key === "Escape";
}

export function shouldHandleShortcutCapture(key: string, isCaptureTarget: boolean) {
  return isCaptureTarget && !shouldBypassShortcutCapture(key);
}

export function isWindowLifecycleShortcut(key: string, altKey: boolean) {
  return altKey && key.toUpperCase() === "F4";
}

export const ShortcutCaptureField = forwardRef<ShortcutCaptureFieldHandle, ShortcutCaptureFieldProps>(function ShortcutCaptureField({
  value,
  label,
  placeholder = "按下快捷键",
  onChange,
  onAppendToken,
  onCaptureStart,
  onCaptureStop,
  autoFocus = false
}, forwardedRef) {
  const tokens = value.trim().split(/\s+/).filter(Boolean);
  const [pendingModifiers, setPendingModifiers] = useState<string[]>([]);
  const pendingModifiersRef = useRef<string[]>([]);
  const modifierChordConsumedRef = useRef(false);
  const windowLifecycleF4ReleaseRef = useRef(false);
  const windowLifecycleAltReleaseRef = useRef(false);
  const captureRef = useRef<HTMLDivElement>(null);

  const setPending = (next: string[] | ((current: string[]) => string[])) => {
    const value = typeof next === "function" ? next(pendingModifiersRef.current) : next;
    pendingModifiersRef.current = value;
    setPendingModifiers(value);
  };

  const appendToken = (token: string) => {
    if (onAppendToken) {
      onAppendToken(token);
    } else {
      onChange([...tokens, token].join(" "));
    }
  };

  const consumePendingModifiers = () => {
    if (pendingModifiersRef.current.length > 0) {
      modifierChordConsumedRef.current = true;
      setPending([]);
    }
  };

  useImperativeHandle(forwardedRef, () => ({
    acceptNativeToken(token: string) {
      consumePendingModifiers();
      appendToken(token);
    }
  }));

  useEffect(() => {
    if (!autoFocus) {
      return undefined;
    }
    const frame = window.requestAnimationFrame(() => captureRef.current?.focus());
    return () => window.cancelAnimationFrame(frame);
  }, [autoFocus]);

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (!shouldHandleShortcutCapture(event.key, event.target === event.currentTarget)) {
      return;
    }

    if (event.key === "Alt") {
      setPending((current) => (current.includes("Alt") ? current : [...current, "Alt"]));
      return;
    }

    if (isWindowLifecycleShortcut(event.key, event.altKey)) {
      windowLifecycleF4ReleaseRef.current = true;
      windowLifecycleAltReleaseRef.current = true;
      modifierChordConsumedRef.current = true;
      setPending([]);
      return;
    }

    event.preventDefault();
    event.stopPropagation();

    const chordHasModifier = pendingModifiersRef.current.length > 0
      || event.ctrlKey
      || event.altKey
      || event.shiftKey
      || event.metaKey;

    if (event.key === "Backspace" && !chordHasModifier) {
      onChange(tokens.slice(0, -1).join(" "));
      return;
    }

    if (event.key === "Delete" && !chordHasModifier) {
      onChange("");
      return;
    }

    if (event.repeat) {
      return;
    }

    const modifierLabel = modifierKeyLabels[event.key];
    if (modifierLabel) {
      setPending((current) => (current.includes(modifierLabel) ? current : [...current, modifierLabel]));
      return;
    }

    const nextToken = normalizeShortcutKey(event);
    if (!nextToken) {
      return;
    }

    if (pendingModifiers.length > 0 || event.ctrlKey || event.altKey || event.shiftKey || event.metaKey) {
      modifierChordConsumedRef.current = true;
      setPending([]);
    }

    appendToken(nextToken);
  };

  const handleKeyUp = (event: KeyboardEvent<HTMLDivElement>) => {
    if (!shouldHandleShortcutCapture(event.key, event.target === event.currentTarget)) {
      return;
    }

    if (
      isWindowLifecycleShortcut(event.key, event.altKey) ||
      (windowLifecycleF4ReleaseRef.current && event.key.toUpperCase() === "F4")
    ) {
      windowLifecycleF4ReleaseRef.current = false;
      return;
    }

    if (windowLifecycleAltReleaseRef.current && event.key === "Alt") {
      windowLifecycleAltReleaseRef.current = false;
      modifierChordConsumedRef.current = false;
      setPending((current) => current.filter((label) => label !== "Alt"));
      return;
    }

    event.preventDefault();
    event.stopPropagation();

    const modifierLabel = modifierKeyLabels[event.key];
    if (!modifierLabel) {
      return;
    }

    setPending((current) => current.filter((label) => label !== modifierLabel));

    // A modifier without a main key cannot be registered as a global
    // shortcut. Keep it as transient guidance instead of creating an
    // unsaveable shortcut block.

    if (!event.ctrlKey && !event.altKey && !event.shiftKey && !event.metaKey) {
      modifierChordConsumedRef.current = false;
    }
  };

  const removeAt = (index: number) => {
    onChange(tokens.filter((_, tokenIndex) => tokenIndex !== index).join(" "));
    window.requestAnimationFrame(() => captureRef.current?.focus());
  };
  const handleCaptureFocus = (event: FocusEvent<HTMLDivElement>) => {
    if (event.target === event.currentTarget) {
      onCaptureStart?.();
    }
  };
  const resetCaptureTransientState = () => {
    pendingModifiersRef.current = [];
    setPendingModifiers([]);
    modifierChordConsumedRef.current = false;
    windowLifecycleF4ReleaseRef.current = false;
    windowLifecycleAltReleaseRef.current = false;
  };
  const handleCaptureBlur = (event: FocusEvent<HTMLDivElement>) => {
    if (event.target === event.currentTarget) {
      resetCaptureTransientState();
      onCaptureStop?.();
    }
  };
  const visiblePendingModifiers = modifierChordConsumedRef.current ? [] : pendingModifiers;

  return (
    <div
      ref={captureRef}
      className={styles.shortcutCapture}
      role="group"
      tabIndex={0}
      aria-label={label}
      onFocus={handleCaptureFocus}
      onBlur={handleCaptureBlur}
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
});
