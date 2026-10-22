import { isValidElement, useRef, useState, type ReactElement } from "react";
import styles from "./Tooltip.module.css";

export type TooltipPlacement = "above" | "below";

export interface TooltipProps {
  /** What the control does, in a few words. An icon-only control carries it as its `aria-label` too. */
  text: string;
  /** Above by default; below where there is no room above, as in a conversation's header. */
  placement?: TooltipPlacement;
  /** The one control it explains. */
  children: ReactElement<{ "aria-expanded"?: boolean | "true" | "false" }>;
}

/**
 * A short label over a control, shown while the mouse is on it or the
 * keyboard's focus is (a focus a mouse press gave shows nothing more). It
 * never shows while the control's menu is open. It is not read out: the
 * control is already named, so the label would only repeat it.
 */
export function Tooltip({ text, placement = "above", children }: TooltipProps) {
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const pressed = useRef(false);
  const expanded = isValidElement(children) ? children.props["aria-expanded"] : undefined;
  const menuOpen = expanded === true || expanded === "true";
  const shown = (hovered || focused) && !menuOpen;
  return (
    <span
      className={styles.anchor}
      onBlur={() => {
        pressed.current = false;
        setFocused(false);
      }}
      onFocus={() => setFocused(!pressed.current)}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      onPointerDown={() => {
        pressed.current = true;
      }}
    >
      {children}
      {shown && (
        <span aria-hidden="true" className={styles.tip} data-placement={placement} data-tooltip="">
          {text}
        </span>
      )}
    </span>
  );
}
