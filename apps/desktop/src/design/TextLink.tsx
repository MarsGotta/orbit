import type { AnchorHTMLAttributes, ReactNode } from "react";
import styles from "./TextLink.module.css";

export interface TextLinkProps extends AnchorHTMLAttributes<HTMLAnchorElement> {
  children: ReactNode;
}

export function TextLink({ children, className, ...props }: TextLinkProps) {
  return (
    <a className={[styles.link, className].filter(Boolean).join(" ")} {...props}>
      {children}
    </a>
  );
}
