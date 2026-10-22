import { useEffect, useId, useRef, type ReactNode } from "react";
import styles from "./Popover.module.css";

export interface PopoverProps {
  /** What the anchored window is for: «Cambiar quién habla». */
  label: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** The content of the control it hangs from. */
  trigger: ReactNode;
  triggerClassName?: string;
  /** Which edge of the control it lines up with. */
  align?: "start" | "end";
  children: ReactNode;
}

const FOCUSABLE = "input:not(:disabled), button:not(:disabled), [href], textarea, select";

/**
 * A menu or small window anchored to the control that opens it. Esc or a
 * press outside closes it; Esc gives the focus back to the control. A large
 * change is confirmed as the last step inside it, not in another window.
 */
export function Popover(props: PopoverProps) {
  const { label, open, onOpenChange, trigger, triggerClassName, align = "start", children } = props;
  const id = useId();
  const anchor = useRef<HTMLSpanElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    panel.current?.querySelector<HTMLElement>(FOCUSABLE)?.focus();
    const outside = (event: MouseEvent) => {
      if (event.target instanceof Node && !anchor.current?.contains(event.target))
        onOpenChange(false);
    };
    document.addEventListener("mousedown", outside);
    return () => document.removeEventListener("mousedown", outside);
  }, [open, onOpenChange]);
  return (
    <span className={styles.anchor} ref={anchor}>
      <button
        aria-controls={open ? id : undefined}
        aria-expanded={open}
        className={triggerClassName ?? styles.trigger}
        onClick={() => onOpenChange(!open)}
        ref={button}
        type="button"
      >
        {trigger}
      </button>
      {open && (
        <div
          aria-label={label}
          className={`${styles.panel} ${align === "end" ? styles.end : ""}`}
          id={id}
          onKeyDown={(event) => {
            if (event.key !== "Escape") return;
            event.stopPropagation();
            onOpenChange(false);
            button.current?.focus();
          }}
          ref={panel}
          role="dialog"
        >
          {children}
        </div>
      )}
    </span>
  );
}
