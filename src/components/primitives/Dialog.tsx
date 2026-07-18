import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { Button } from "./Button";
import { TextField } from "./Field";
import styles from "./primitives.module.css";

type DialogShellProps = {
  open: boolean;
  title: string;
  description?: string;
  children?: ReactNode;
  footer: ReactNode;
  onClose: () => void;
};

type DialogReturnFocusTarget = {
  isConnected: boolean;
  focus: () => void;
};

export function restoreDialogFocus(target: DialogReturnFocusTarget | null) {
  if (target?.isConnected) {
    target.focus();
  }
}

const dialogFocusableSelector = [
  "a[href]",
  "area[href]",
  "button:not([disabled])",
  "input:not([disabled]):not([type='hidden'])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  "iframe",
  "object",
  "embed",
  "[contenteditable='true']",
  "[tabindex]:not([tabindex='-1'])"
].join(",");

function getDialogFocusableElements(dialog: HTMLElement) {
  return Array.from(dialog.querySelectorAll<HTMLElement>(dialogFocusableSelector)).filter((element) => {
    const style = window.getComputedStyle(element);
    return element.tabIndex >= 0
      && !element.closest("[hidden], [aria-hidden='true']")
      && style.display !== "none"
      && style.visibility !== "hidden";
  });
}

export function DialogShell({ open, title, description, children, footer, onClose }: DialogShellProps) {
  const titleId = useId();
  const descriptionId = useId();
  const overlayRef = useRef<HTMLDivElement>(null);
  const dialogRef = useRef<HTMLElement>(null);

  useEffect(() => {
    if (!open) {
      return undefined;
    }

    const returnFocus = document.activeElement instanceof HTMLElement
      ? document.activeElement
      : null;
    const overlay = overlayRef.current;
    const backgroundStates = overlay
      ? Array.from(document.body.children)
        .filter((element): element is HTMLElement => element instanceof HTMLElement && element !== overlay)
        .map((element) => ({
          element,
          inert: element.inert === true,
          ariaHidden: element.getAttribute("aria-hidden")
        }))
      : [];

    backgroundStates.forEach(({ element }) => {
      element.inert = true;
      element.setAttribute("aria-hidden", "true");
    });

    const focusFrame = window.requestAnimationFrame(() => {
      const dialog = dialogRef.current;
      if (!dialog || dialog.contains(document.activeElement)) {
        return;
      }
      const [firstFocusable] = getDialogFocusableElements(dialog);
      (firstFocusable ?? dialog).focus();
    });

    return () => {
      window.cancelAnimationFrame(focusFrame);
      backgroundStates.forEach(({ element, inert, ariaHidden }) => {
        element.inert = inert;
        if (ariaHidden === null) {
          element.removeAttribute("aria-hidden");
        } else {
          element.setAttribute("aria-hidden", ariaHidden);
        }
      });
      restoreDialogFocus(returnFocus);
    };
  }, [open]);

  useEffect(() => {
    if (!open) {
      return undefined;
    }

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        onClose();
        return;
      }

      if (event.key !== "Tab") {
        return;
      }

      const dialog = dialogRef.current;
      if (!dialog) {
        return;
      }

      const focusableElements = getDialogFocusableElements(dialog);
      if (focusableElements.length === 0) {
        event.preventDefault();
        dialog.focus();
        return;
      }

      const firstFocusable = focusableElements[0];
      const lastFocusable = focusableElements[focusableElements.length - 1];
      const activeElement = document.activeElement;

      if (event.shiftKey && (activeElement === firstFocusable || !dialog.contains(activeElement))) {
        event.preventDefault();
        lastFocusable.focus();
      } else if (!event.shiftKey && (activeElement === lastFocusable || !dialog.contains(activeElement))) {
        event.preventDefault();
        firstFocusable.focus();
      }
    };

    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [onClose, open]);

  if (!open) {
    return null;
  }

  return createPortal(
    <div ref={overlayRef} className={styles.dialogOverlay} role="presentation" onMouseDown={onClose}>
      <section
        ref={dialogRef}
        className={styles.dialogSurface}
        role="dialog"
        tabIndex={-1}
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={description ? descriptionId : undefined}
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header className={styles.dialogHeader}>
          <div>
            <h2 className={styles.dialogTitle} id={titleId}>
              {title}
            </h2>
            {description && (
              <p className={styles.dialogDescription} id={descriptionId}>
                {description}
              </p>
            )}
          </div>
          <button className={styles.dialogClose} type="button" aria-label="关闭弹窗" onClick={onClose}>
            ×
          </button>
        </header>
        {children && <div className={styles.dialogBody}>{children}</div>}
        <footer className={styles.dialogFooter}>{footer}</footer>
      </section>
    </div>,
    document.body
  );
}

type NoticeDialogProps = {
  open: boolean;
  title: string;
  description?: string;
  children?: ReactNode;
  onClose: () => void;
};

export function NoticeDialog({ open, title, description, children, onClose }: NoticeDialogProps) {
  return (
    <DialogShell
      open={open}
      title={title}
      description={description}
      onClose={onClose}
      footer={
        <Button size="inline" onClick={onClose}>
          知道了
        </Button>
      }
    >
      {children}
    </DialogShell>
  );
}

type ConfirmDialogProps = {
  open: boolean;
  title: string;
  description?: string;
  confirmText?: string;
  cancelText?: string;
  danger?: boolean;
  onConfirm: () => void;
  onClose: () => void;
};

export function ConfirmDialog({
  open,
  title,
  description,
  confirmText = "确认",
  cancelText = "取消",
  danger = false,
  onConfirm,
  onClose
}: ConfirmDialogProps) {
  return (
    <DialogShell
      open={open}
      title={title}
      description={description}
      onClose={onClose}
      footer={
        <>
          <Button size="inline" onClick={onClose}>
            {cancelText}
          </Button>
          <Button className={danger ? styles.dialogDangerButton : ""} size="inline" onClick={onConfirm}>
            {confirmText}
          </Button>
        </>
      }
    >
      {null}
    </DialogShell>
  );
}

type InputDialogField = {
  name: string;
  label: string;
  value: string;
  placeholder?: string;
};

type InputDialogProps = {
  open: boolean;
  title: string;
  description?: string;
  fields: InputDialogField[];
  confirmText?: string;
  onChange: (name: string, value: string) => void;
  onConfirm: () => void;
  onClose: () => void;
};

export function InputDialog({
  open,
  title,
  description,
  fields,
  confirmText = "确定",
  onChange,
  onConfirm,
  onClose
}: InputDialogProps) {
  const [touched, setTouched] = useState(false);
  const canConfirm = fields.every((field) => field.value.trim().length > 0);

  useEffect(() => {
    if (open) {
      setTouched(false);
    }
  }, [open]);

  return (
    <DialogShell
      open={open}
      title={title}
      description={description}
      onClose={onClose}
      footer={
        <>
          <Button size="inline" onClick={onClose}>
            取消
          </Button>
          <Button
            size="inline"
            onClick={() => {
              setTouched(true);
              if (canConfirm) {
                onConfirm();
              }
            }}
          >
            {confirmText}
          </Button>
        </>
      }
    >
      <div className={styles.dialogForm}>
        {fields.map((field) => (
          <label className={styles.dialogField} key={field.name}>
            <span>{field.label}</span>
            <TextField
              value={field.value}
              placeholder={field.placeholder}
              onChange={(event) => onChange(field.name, event.target.value)}
            />
          </label>
        ))}
        {touched && !canConfirm && <div className={styles.dialogError}>请填写完整内容。</div>}
      </div>
    </DialogShell>
  );
}
