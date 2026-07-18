import { useEffect, useId, useState, type ReactNode } from "react";
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

export function DialogShell({ open, title, description, children, footer, onClose }: DialogShellProps) {
  const titleId = useId();
  const descriptionId = useId();

  useEffect(() => {
    if (!open) {
      return undefined;
    }

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    };

    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [onClose, open]);

  if (!open) {
    return null;
  }

  return createPortal(
    <div className={styles.dialogOverlay} role="presentation" onMouseDown={onClose}>
      <section
        className={styles.dialogSurface}
        role="dialog"
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
