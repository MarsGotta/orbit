import { isValidElement, useLayoutEffect, useRef, useState, type ReactElement } from "react";
import { createPortal } from "react-dom";
import styles from "./Tooltip.module.css";

export type TooltipPlacement = "above" | "below";

// 6 points from its control, and at least 8 from the window's edges.
const GAP = 6;
const MARGIN = 8;

export interface TooltipProps {
  /** What the control does, in a few words. An icon-only control carries it as its `aria-label` too. */
  text: string;
  /** Above by default; below where there is no room above, as in a conversation's header. */
  placement?: TooltipPlacement;
  /** The one control it explains. */
  children: ReactElement<{ "aria-expanded"?: boolean | "true" | "false" }>;
}

type Spot = { placement: TooltipPlacement; top: number; left: number };

/**
 * On its side when it fits there, on the other when only that one fits;
 * centred on its control and moved in from the window's sides. A tip not laid
 * out yet (no size) has no spot: it keeps the side it was asked for.
 */
function spot(control: DOMRect, tip: DOMRect, wanted: TooltipPlacement): Spot | null {
  if (tip.width === 0 && tip.height === 0) return null;
  const fitsAbove = control.top - GAP - tip.height >= MARGIN;
  const fitsBelow = control.bottom + GAP + tip.height <= window.innerHeight - MARGIN;
  const above = wanted === "above" ? fitsAbove || !fitsBelow : !fitsBelow && fitsAbove;
  const centred = control.left + control.width / 2 - tip.width / 2;
  return {
    placement: above ? "above" : "below",
    top: above ? control.top - GAP - tip.height : control.bottom + GAP,
    left: Math.max(MARGIN, Math.min(centred, window.innerWidth - MARGIN - tip.width)),
  };
}

/** Where the shown tip goes, measured again while the page scrolls or the window changes. */
function useSpot(shown: boolean, wanted: TooltipPlacement, text: string) {
  const anchor = useRef<HTMLSpanElement>(null);
  const tip = useRef<HTMLSpanElement>(null);
  const [at, setAt] = useState<Spot | null>(null);
  useLayoutEffect(() => {
    if (!shown) return;
    const update = () => {
      if (!anchor.current || !tip.current) return;
      const next = spot(
        anchor.current.getBoundingClientRect(),
        tip.current.getBoundingClientRect(),
        wanted,
      );
      setAt((now) => (JSON.stringify(now) === JSON.stringify(next) ? now : next));
    };
    update();
    window.addEventListener("resize", update);
    window.addEventListener("scroll", update, true);
    return () => {
      window.removeEventListener("resize", update);
      window.removeEventListener("scroll", update, true);
    };
  }, [shown, wanted, text]);
  return { anchor, tip, at };
}

/**
 * A short label over a control, shown while the mouse is on it or the
 * keyboard's focus is (a focus a mouse press gave shows nothing more). It
 * never shows while the control's menu is open. It is not read out: the
 * control is already named, so the label would only repeat it. It is drawn
 * over the whole window, so no box its control is in clips it, and it stays
 * inside the window at its edges.
 */
export function Tooltip({ text, placement = "above", children }: TooltipProps) {
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const pressed = useRef(false);
  const expanded = isValidElement(children) ? children.props["aria-expanded"] : undefined;
  const menuOpen = expanded === true || expanded === "true";
  const shown = (hovered || focused) && !menuOpen;
  const { anchor, tip, at } = useSpot(shown, placement, text);
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
      ref={anchor}
    >
      {children}
      {shown &&
        createPortal(
          <span
            aria-hidden="true"
            className={styles.tip}
            data-placement={at?.placement ?? placement}
            data-tooltip=""
            ref={tip}
            style={at === null ? undefined : { top: at.top, left: at.left }}
          >
            {text}
          </span>,
          document.body,
        )}
    </span>
  );
}
