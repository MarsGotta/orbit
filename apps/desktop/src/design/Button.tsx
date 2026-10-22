import { useId, type ButtonHTMLAttributes, type MouseEvent, type ReactNode } from "react";
import { es } from "../i18n/es";
import type { Feedback } from "./feedback";
import { Spinner } from "./Spinner";
import styles from "./Button.module.css";

/**
 * `accent` is the one main action of a view; `outline` the alternatives;
 * `ghost` the quiet ones (Cancelar, row actions); `danger` only «Empezar a
 * grabar», «Grabar» in the panel and «Colgar»; `dangerSoft` «Parar todo»,
 * «Terminar», «Borrar»; `icon` a square button with only an icon (give it an `aria-label`).
 */
export type ButtonVariant = "accent" | "outline" | "ghost" | "danger" | "dangerSoft" | "icon";
/** 28 in bars and notices, 32 in cards, 36 standard, 40 for a view's main action. */
export type ButtonSize = 28 | 32 | 36 | 40;

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  children: ReactNode;
  size?: ButtonSize;
  variant?: ButtonVariant;
  /** Why it cannot be pressed now, shown beside it: a disabled button always says why. */
  disabledReason?: string;
  /** What the action is doing; see `useFeedback`. */
  feedback?: Feedback;
  pendingLabel?: string;
  doneLabel?: string;
}

export function Button({
  children,
  className,
  size = 28,
  type = "button",
  variant = "outline",
  disabledReason,
  feedback,
  pendingLabel = es.controls.saving,
  doneLabel = es.controls.saved,
  onClick,
  disabled,
  "aria-describedby": describedBy,
  ...props
}: ButtonProps) {
  const reasonId = useId();
  const classes = [styles.button, styles[variant], styles[`size${size}`], className]
    .filter(Boolean)
    .join(" ");
  const pending = feedback === "pending";
  const button = (
    <button
      aria-busy={pending || undefined}
      aria-describedby={disabledReason ? reasonId : describedBy}
      aria-disabled={pending || undefined}
      className={classes}
      disabled={disabledReason !== undefined || disabled}
      onClick={(event: MouseEvent<HTMLButtonElement>) => {
        if (!pending) onClick?.(event);
      }}
      type={type}
      {...props}
    >
      {feedback === undefined ? (
        children
      ) : (
        <Labels done={doneLabel} feedback={feedback} pending={pendingLabel}>
          {children}
        </Labels>
      )}
    </button>
  );
  if (!disabledReason) return button;
  return (
    <span className={styles.withReason}>
      {button}
      <span className={styles.reason} id={reasonId}>
        {disabledReason}
      </span>
    </span>
  );
}

/** The three labels in one cell: the widest sets the width, only the current one shows. */
function Labels(props: { children: ReactNode; feedback: Feedback; pending: string; done: string }) {
  const { children, feedback, pending, done } = props;
  return (
    <span aria-live="polite" className={styles.labels}>
      <span aria-hidden={feedback !== "idle"} data-shown={feedback === "idle"}>
        {children}
      </span>
      <span aria-hidden={feedback !== "pending"} data-shown={feedback === "pending"}>
        <Spinner size={12} />
        {pending}
      </span>
      <span
        aria-hidden={feedback !== "done"}
        className={styles.done}
        data-shown={feedback === "done"}
      >
        {done}
      </span>
    </span>
  );
}

/** «Grabar reunión» in the sidebar: secondary, with the red dot and its shortcut. */
export function RecordButton({
  children,
  shortcut,
  ...props
}: Omit<ButtonProps, "variant"> & { shortcut?: string }) {
  return (
    <Button size={36} variant="outline" {...props}>
      <span aria-hidden="true" className={styles.recordDot} />
      {children}
      {shortcut && (
        <kbd aria-hidden="true" className={styles.shortcut}>
          {shortcut}
        </kbd>
      )}
    </Button>
  );
}
