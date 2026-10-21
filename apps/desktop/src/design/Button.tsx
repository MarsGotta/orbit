import type { ButtonHTMLAttributes, ReactNode } from "react";
import styles from "./Button.module.css";

export type ButtonVariant = "accent" | "outline" | "ghost" | "icon";
export type ButtonSize = 26 | 28 | 30 | 34;

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  children: ReactNode;
  size?: ButtonSize;
  variant?: ButtonVariant;
}

export function Button({
  children,
  className,
  size = 28,
  type = "button",
  variant = "outline",
  ...props
}: ButtonProps) {
  const classes = [styles.button, styles[variant], styles[`size${size}`], className]
    .filter(Boolean)
    .join(" ");
  return (
    <button className={classes} type={type} {...props}>
      {children}
    </button>
  );
}
