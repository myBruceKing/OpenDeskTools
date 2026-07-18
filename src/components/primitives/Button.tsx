import type { ButtonHTMLAttributes, ReactNode } from "react";
import styles from "./primitives.module.css";

type ButtonVariant = "outline" | "primary" | "text";
type ButtonSize = "default" | "compact" | "inline" | "footer";

type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: ButtonVariant;
  size?: ButtonSize;
  icon?: ReactNode;
};

export function Button({
  variant = "outline",
  size = "default",
  icon,
  className = "",
  children,
  type = "button",
  ...props
}: ButtonProps) {
  const classes = [
    styles.button,
    variant === "primary" ? styles.primary : "",
    variant === "text" ? styles.text : "",
    size === "compact" ? styles.compact : "",
    size === "inline" ? styles.inline : "",
    size === "footer" ? styles.footer : "",
    className
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <button className={classes} type={type} {...props}>
      {icon}
      {children}
    </button>
  );
}
