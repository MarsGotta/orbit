import type { InputHTMLAttributes } from "react";
import styles from "./TextInput.module.css";

export type TextInputVariant = "transparent" | "dotted";

export interface TextInputProps extends InputHTMLAttributes<HTMLInputElement> {
  variant?: TextInputVariant;
}

export function TextInput({ className, variant = "transparent", ...props }: TextInputProps) {
  return <input className={`${styles.input} ${styles[variant]} ${className ?? ""}`} {...props} />;
}
