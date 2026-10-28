import type { ButtonHTMLAttributes, Ref } from "react";
import type { Feedback } from "./feedback";
import { Icon, type IconName } from "./Icon";
import { Tooltip, type TooltipPlacement } from "./Tooltip";
import styles from "./IconButton.module.css";

/** 28 under an answer, 30 for a header's pencil and gear, 34 for a header's call and theme. */
export type IconButtonSize = 28 | 30 | 34;

export interface IconButtonProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "children"> {
  icon: IconName;
  /** Its tooltip, and its name unless `aria-label` says more («… de Phobos»). */
  label: string;
  size: IconButtonSize;
  tip?: TooltipPlacement;
  /** Copy's echo: tracked with `useFeedback`, it turns to ✓ and `doneLabel` once done. */
  feedback?: Feedback;
  doneLabel?: string;
  /** Listen's echo: while what it started goes on, it shows this, in the accent. */
  active?: { icon: IconName; label: string };
  /** The control a menu hangs from needs to be found. */
  ref?: Ref<HTMLButtonElement>;
}

/** A square button with only an icon and a tooltip that names it. */
export function IconButton({
  icon,
  label,
  size,
  tip,
  feedback,
  doneLabel,
  active,
  className,
  onClick,
  type = "button",
  "aria-label": name,
  ...props
}: IconButtonProps) {
  const done = feedback === "done" && doneLabel !== undefined;
  const echo = done ? "done" : active ? "active" : undefined;
  const shownLabel = done ? doneLabel : (active?.label ?? label);
  const shownIcon = done ? "check" : (active?.icon ?? icon);
  const pending = feedback === "pending";
  return (
    <Tooltip placement={tip} text={shownLabel}>
      <button
        aria-busy={pending || undefined}
        aria-label={echo ? shownLabel : (name ?? label)}
        className={[styles.button, styles[`size${size}`], className].filter(Boolean).join(" ")}
        data-echo={echo}
        onClick={(event) => {
          if (!pending) onClick?.(event);
        }}
        type={type}
        {...props}
      >
        <Icon name={shownIcon} size={size === 34 ? 16 : 15} />
        {feedback !== undefined && (
          <span className={styles.announce} role="status">
            {done ? doneLabel : ""}
          </span>
        )}
      </button>
    </Tooltip>
  );
}
